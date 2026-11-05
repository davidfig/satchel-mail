//! Calendar alerts as macOS notifications (0057), through the User
//! Notifications framework: upcoming alerts are scheduled with the system, so
//! they show even while the app is quit, with Snooze and Dismiss actions;
//! clicking one opens the event. The core still decides which alerts there
//! are; this keeps the system's schedule in step with it.
//!
//! The framework only works in an app bundle. A bare binary (`tauri dev`)
//! falls back to the notification plugin (see `notify.rs`).

use std::collections::{HashMap, HashSet};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use block2::RcBlock;
use chrono::{Local, Utc};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification, UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
    UNNotificationCategoryOptions, UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNNotificationSettings, UNNotificationSound, UNTimeIntervalNotificationTrigger, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use satchel_api::{CalendarAlert, Command, CoreEvent};
use satchel_core::{Caller, Engine};
use tauri::{Emitter, Manager};
use tokio::sync::{Notify, oneshot};
use tracing::{debug, warn};

/// Identifiers of alert notifications: `alert/{alert id}@{due, ms}`.
const PREFIX: &str = "alert/";
const CATEGORY: &str = "satchel.alert";
/// How far ahead alerts are scheduled, and how many (macOS keeps 64 per app).
const AHEAD: chrono::Duration = chrono::Duration::days(7);
const MAX_SCHEDULED: usize = 50;
/// Resync at least this often, so the week ahead keeps moving.
const RESYNC: Duration = Duration::from_secs(15 * 60);

struct Host {
    app: tauri::AppHandle,
    engine: Arc<Engine>,
    /// Identifiers handed to the system to show at their time.
    scheduled: Mutex<HashSet<String>>,
    resync: Notify,
    /// Whether the main window has focus: alerts then show in the app instead.
    focused: AtomicBool,
    /// An event to open, from a notification clicked before the UI was up.
    open: Mutex<Option<serde_json::Value>>,
}

static HOST: OnceLock<Host> = OnceLock::new();

/// Whether this process can use the framework: it's running from an app bundle.
pub fn available() -> bool {
    NSBundle::mainBundle().bundlePath().to_string().ends_with(".app")
}

fn center() -> Retained<UNUserNotificationCenter> {
    UNUserNotificationCenter::currentNotificationCenter()
}

fn ns_array(items: &[String]) -> Retained<NSArray<NSString>> {
    let strings: Vec<Retained<NSString>> = items.iter().map(|s| NSString::from_str(s)).collect();
    NSArray::from_retained_slice(&strings)
}

/// Starts keeping the system's schedule in step with the core. Call once,
/// during setup, before the app finishes launching: a notification clicked
/// while the app was quit is delivered to the delegate set here.
pub fn start(app: &tauri::AppHandle, engine: Arc<Engine>) {
    let host = Host { app: app.clone(), engine, scheduled: Mutex::default(), resync: Notify::new(), focused: AtomicBool::new(true), open: Mutex::default() };
    if HOST.set(host).is_err() {
        return;
    }
    let c = center();
    // The center holds its delegate weakly; this one lives as long as the app.
    let delegate: &'static Delegate = Box::leak(Box::new(Delegate::new()));
    c.setDelegate(Some(ProtocolObject::from_ref(delegate)));
    let action = |id: &str, title: &str, options: UNNotificationActionOptions| UNNotificationAction::actionWithIdentifier_title_options(&NSString::from_str(id), &NSString::from_str(title), options);
    let none = UNNotificationActionOptions::empty();
    let actions = NSArray::from_retained_slice(&[
        action("snooze.5", "Snooze 5 minutes", none),
        action("snooze.15", "Snooze 15 minutes", none),
        action("snooze.60", "Snooze 1 hour", none),
        action("dismiss", "Dismiss", none),
    ]);
    // CustomDismissAction: closing the notification dismisses the alert too.
    let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(&NSString::from_str(CATEGORY), &actions, &NSArray::new(), UNNotificationCategoryOptions::CustomDismissAction);
    c.setNotificationCategories(&NSSet::from_retained_slice(&[category]));
    tauri::async_runtime::spawn(watch());
    tauri::async_runtime::spawn(keep_in_step());
}

/// The main window gained or lost focus.
pub fn set_focused(focused: bool) {
    if let Some(h) = HOST.get() {
        h.focused.store(focused, Ordering::Relaxed);
    }
}

/// The event a notification asked to open before the UI was listening.
pub fn take_open_request() -> Option<serde_json::Value> {
    HOST.get()?.open.lock().unwrap().take()
}

/// Reacts to the core: shows alerts the system wasn't given in time, takes
/// down ones that stopped showing, and reschedules after changes.
async fn watch() {
    let Some(h) = HOST.get() else { return };
    let mut rx = h.engine.subscribe();
    loop {
        match rx.recv().await {
            Ok(CoreEvent::AlertsChanged { due, closed }) => {
                show_missed(&due).await;
                take_down(&closed).await;
                h.resync.notify_one();
            }
            // Not DataChanged: it follows every write, this process's too.
            Ok(CoreEvent::CalendarChanged { .. }) => h.resync.notify_one(),
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => h.resync.notify_one(),
            Err(_) => return,
        }
    }
}

async fn keep_in_step() {
    let Some(h) = HOST.get() else { return };
    loop {
        if let Err(e) = resync(h).await {
            warn!("alerts: {e:#}");
        }
        tokio::select! {
            _ = tokio::time::sleep(RESYNC) => {},
            _ = h.resync.notified() => {},
        }
        // Changes come in bursts (a sync stores many events).
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

fn notification_id(a: &CalendarAlert) -> String {
    format!("{PREFIX}{}@{}", a.id, a.due.timestamp_millis())
}

/// The alert id in a notification's identifier.
fn alert_id(identifier: &str) -> Option<&str> {
    identifier.strip_prefix(PREFIX)?.rsplit_once('@').map(|(id, _)| id)
}

async fn resync(h: &Host) -> anyhow::Result<()> {
    let until = Utc::now() + AHEAD;
    let v = h.engine.handle(&Caller::User, Command::ListUpcomingAlerts { until }).await.map_err(|e| anyhow::anyhow!(e.message))?;
    let upcoming: Vec<CalendarAlert> = serde_json::from_value(v)?;
    let want: HashMap<String, &CalendarAlert> = upcoming.iter().take(MAX_SCHEDULED).map(|a| (notification_id(a), a)).collect();
    if !want.is_empty() {
        ask_permission().await;
    }
    let pending: HashSet<String> = pending_ids().await.into_iter().filter(|id| id.starts_with(PREFIX)).collect();
    let stale: Vec<String> = pending.iter().filter(|id| !want.contains_key(*id)).cloned().collect();
    let c = center();
    if !stale.is_empty() {
        c.removePendingNotificationRequestsWithIdentifiers(&ns_array(&stale));
    }
    for (id, a) in &want {
        if !pending.contains(id) {
            let wait = (a.due - Utc::now()).num_milliseconds().max(1000) as f64 / 1000.0;
            post(id, a, Some(wait));
        }
    }
    debug!("alerts: {} scheduled with macOS, {} taken off", want.len(), stale.len());
    *h.scheduled.lock().unwrap() = want.into_keys().collect();
    Ok(())
}

/// Alerts the core says just went off that the system wasn't scheduled to
/// show (they came up while the app was quit, or after the last resync).
async fn show_missed(due: &[CalendarAlert]) {
    let Some(h) = HOST.get() else { return };
    if due.is_empty() || !ask_permission().await {
        return;
    }
    let delivered = delivered_ids().await;
    for a in due {
        let id = notification_id(a);
        let scheduled = h.scheduled.lock().unwrap().contains(&id);
        debug!("alert {id} went off: scheduled {scheduled}, shown by macOS {}", delivered.contains(&id));
        if !scheduled && !delivered.contains(&id) {
            post(&id, a, None);
        }
    }
}

/// Takes down the notifications of alerts dismissed or snoozed elsewhere.
async fn take_down(closed: &[String]) {
    if closed.is_empty() {
        return;
    }
    let gone: Vec<String> = delivered_ids().await.into_iter().filter(|id| alert_id(id).is_some_and(|a| closed.iter().any(|c| c == a))).collect();
    if !gone.is_empty() {
        center().removeDeliveredNotificationsWithIdentifiers(&ns_array(&gone));
    }
}

/// "In 10 minutes", "Now", "Started 5 minutes ago": as of when it shows.
fn when(a: &CalendarAlert) -> String {
    if a.all_day {
        return "All day".into();
    }
    let span = |m: i64| match m {
        m if m % 1440 == 0 => format!("{} day{}", m / 1440, if m == 1440 { "" } else { "s" }),
        m if m % 60 == 0 => format!("{} hour{}", m / 60, if m == 60 { "" } else { "s" }),
        m => format!("{m} minute{}", if m == 1 { "" } else { "s" }),
    };
    match (a.start - a.due).num_minutes() {
        0 => "Now".into(),
        m if m > 0 => format!("In {}", span(m)),
        m => format!("Started {} ago", span(-m)),
    }
}

/// "2:00 – 3:00 PM", with the day when it isn't the day the alert shows.
fn times(a: &CalendarAlert) -> String {
    let (s, e, due) = (a.start.with_timezone(&Local), a.end.with_timezone(&Local), a.due.with_timezone(&Local));
    if a.all_day {
        // All-day events are UTC midnights.
        return a.start.format("%A, %B %-d").to_string();
    }
    let day = if s.date_naive() == due.date_naive() { String::new() } else { s.format("%a %b %-d, ").to_string() };
    format!("{day}{} – {}", s.format("%-I:%M %p"), e.format("%-I:%M %p"))
}

/// Hands the system a notification for `a`: after `wait` seconds, or now.
fn post(id: &str, a: &CalendarAlert, wait: Option<f64>) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(if a.title.is_empty() { "(no title)" } else { &a.title }));
    content.setSubtitle(&NSString::from_str(&when(a)));
    let body = [times(a), a.location.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    content.setBody(&NSString::from_str(&body));
    content.setCategoryIdentifier(&NSString::from_str(CATEGORY));
    content.setThreadIdentifier(&NSString::from_str("calendar-alerts"));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let trigger = wait.map(|w| UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(w, false));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(id), &content, trigger.as_deref().map(|t| &**t));
    add(&request);
}

/// A plain notification, now (new mail).
pub fn notify(title: &str, body: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let id = format!("mail/{}", Utc::now().timestamp_nanos_opt().unwrap_or_default());
    add(&UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None));
}

fn add(request: &UNNotificationRequest) {
    let done = RcBlock::new(|err: *mut NSError| {
        // SAFETY: the framework passes a valid error or null.
        if let Some(e) = unsafe { err.as_ref() } {
            warn!("notification: {}", e.localizedDescription());
        }
    });
    center().addNotificationRequest_withCompletionHandler(request, Some(&done));
}

async fn pending_ids() -> Vec<String> {
    let rx = {
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let block = RcBlock::new(move |list: NonNull<NSArray<UNNotificationRequest>>| {
            // SAFETY: the framework passes a valid array.
            let ids = unsafe { list.as_ref() }.iter().map(|r| r.identifier().to_string()).collect();
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(ids);
            }
        });
        center().getPendingNotificationRequestsWithCompletionHandler(&block);
        rx
    };
    rx.await.unwrap_or_default()
}

async fn delivered_ids() -> HashSet<String> {
    let rx = {
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let block = RcBlock::new(move |list: NonNull<NSArray<UNNotification>>| {
            // SAFETY: the framework passes a valid array.
            let ids = unsafe { list.as_ref() }.iter().map(|n| n.request().identifier().to_string()).collect();
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(ids);
            }
        });
        center().getDeliveredNotificationsWithCompletionHandler(&block);
        rx
    };
    rx.await.unwrap_or_default()
}

async fn status() -> UNAuthorizationStatus {
    let rx = {
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let block = RcBlock::new(move |s: NonNull<UNNotificationSettings>| {
            // SAFETY: the framework passes valid settings.
            let st = unsafe { s.as_ref() }.authorizationStatus();
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(st);
            }
        });
        center().getNotificationSettingsWithCompletionHandler(&block);
        rx
    };
    rx.await.unwrap_or(UNAuthorizationStatus::NotDetermined)
}

/// Whether notifications may show; asks the user once if they haven't said.
pub async fn ask_permission() -> bool {
    let st = status().await;
    debug!("notification permission: {}", st.0);
    match st {
        UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional => return true,
        UNAuthorizationStatus::Denied => return false,
        _ => {}
    }
    let rx = {
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let block = RcBlock::new(move |granted: Bool, err: *mut NSError| {
            // SAFETY: the framework passes a valid error or null.
            if let Some(e) = unsafe { err.as_ref() } {
                warn!("notifications not allowed: {}", e.localizedDescription());
            }
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(granted.as_bool());
            }
        });
        center().requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &block);
        rx
    };
    rx.await.unwrap_or(false)
}

/// Whether notifications may show, without asking.
pub async fn allowed() -> bool {
    matches!(status().await, UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional)
}

/// What the user did with an alert's notification.
async fn respond(identifier: String, action: String) {
    let Some(h) = HOST.get() else { return };
    let Some(alert) = alert_id(&identifier).map(String::from) else {
        // Not an alert (new mail): bring the app forward.
        show_window(h);
        return;
    };
    let command = match action.as_str() {
        "snooze.5" => Command::SnoozeAlert { alert_id: alert, until: Utc::now() + chrono::Duration::minutes(5) },
        "snooze.15" => Command::SnoozeAlert { alert_id: alert, until: Utc::now() + chrono::Duration::minutes(15) },
        "snooze.60" => Command::SnoozeAlert { alert_id: alert, until: Utc::now() + chrono::Duration::hours(1) },
        "dismiss" | "dismissed" => Command::DismissAlert { alert_id: Some(alert) },
        _ => return open_event(h, &alert),
    };
    if let Err(e) = h.engine.handle(&Caller::User, command).await {
        warn!("alert action {action}: {}", e.message);
    }
}

fn show_window(h: &Host) {
    if let Some(w) = h.app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Brings the app forward on the alert's event.
fn open_event(h: &Host, alert: &str) {
    show_window(h);
    let Some((event_id, start, _)) = satchel_core::calendar::parse_alert_id(alert) else { return };
    let request = serde_json::json!({ "id": event_id, "start": start });
    // The UI asks for it when it starts, in case it isn't up yet.
    *h.open.lock().unwrap() = Some(request.clone());
    let _ = h.app.emit_to("main", "satchel://open-event", request);
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and Delegate has no Drop.
    #[unsafe(super(NSObject))]
    #[name = "SatchelNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// While the app is open: alerts show in the window if it has focus.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _center: &UNUserNotificationCenter, notification: &UNNotification, done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
            let is_alert = notification.request().identifier().to_string().starts_with(PREFIX);
            let focused = HOST.get().is_some_and(|h| h.focused.load(Ordering::Relaxed));
            let show = UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound;
            done.call((if is_alert && focused { UNNotificationPresentationOptions::empty() } else { show },));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
            let identifier = response.notification().request().identifier().to_string();
            let action = response.actionIdentifier();
            // SAFETY: framework constants.
            let action = if &*action == unsafe { UNNotificationDismissActionIdentifier } {
                "dismissed".to_string()
            } else if &*action == unsafe { UNNotificationDefaultActionIdentifier } {
                "open".to_string()
            } else {
                action.to_string()
            };
            tauri::async_runtime::spawn(respond(identifier, action));
            done.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(start_in: i64, due_in: i64) -> CalendarAlert {
        let now = Utc::now();
        CalendarAlert {
            id: "ev1:1700000000000:10".into(),
            event_id: "ev1".into(),
            calendar_id: "cal".into(),
            title: "Review".into(),
            location: String::new(),
            start: now + chrono::Duration::minutes(start_in),
            end: now + chrono::Duration::minutes(start_in + 30),
            all_day: false,
            color: String::new(),
            minutes_before: 10,
            due: now + chrono::Duration::minutes(due_in),
            snoozed: false,
        }
    }

    #[test]
    fn identifiers_and_wording() {
        let a = alert(10, 0);
        let id = notification_id(&a);
        assert!(id.starts_with("alert/ev1:1700000000000:10@"));
        assert_eq!(alert_id(&id), Some("ev1:1700000000000:10"));
        assert_eq!(alert_id("mail/123"), None);
        assert_eq!(when(&a), "In 10 minutes");
        assert_eq!(when(&alert(60, 0)), "In 1 hour");
        assert_eq!(when(&alert(0, 0)), "Now");
        assert_eq!(when(&alert(-5, 0)), "Started 5 minutes ago");
    }
}
