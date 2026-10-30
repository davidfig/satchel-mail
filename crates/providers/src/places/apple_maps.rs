//! Apple Maps place suggestions through MapKit's `MKLocalSearchCompleter`,
//! the completer Apple's own search fields use. It calls its delegate on the
//! main thread, so each query starts there (through the host's `RunOnMain`)
//! and the answer comes back over a channel.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString};
use objc2_map_kit::{MKLocalSearchCompleter, MKLocalSearchCompleterDelegate, MKLocalSearchCompleterResultType};
use satchel_api::{PlaceSource, PlaceSuggestion};
use satchel_core::provider::PlaceSearch;
use tokio::sync::oneshot;

/// Runs a closure on the main thread (Tauri's `run_on_main_thread`).
pub type RunOnMain = Box<dyn Fn(Box<dyn FnOnce() + Send>) -> Result<()> + Send + Sync>;

type Reply = oneshot::Sender<Result<Vec<PlaceSuggestion>>>;

pub struct AppleMaps {
    run_on_main: RunOnMain,
}

impl AppleMaps {
    pub fn new(run_on_main: RunOnMain) -> Self {
        Self { run_on_main }
    }
}

#[async_trait]
impl PlaceSearch for AppleMaps {
    fn name(&self) -> &'static str {
        "Apple Maps"
    }

    async fn suggest(&self, query: &str) -> Result<Vec<PlaceSuggestion>> {
        let (tx, rx) = oneshot::channel();
        let query = query.to_string();
        (self.run_on_main)(Box::new(move || start(&query, tx)))?;
        rx.await.map_err(|_| anyhow!("Apple Maps didn't answer"))?
    }
}

/// Completers waiting for an answer, by query number. MapKit holds its
/// delegate weakly, so both live here until the answer (or a minute) passes.
struct Pending {
    _completer: Retained<MKLocalSearchCompleter>,
    _delegate: Retained<Delegate>,
    started: Instant,
}

thread_local! {
    static PENDING: RefCell<HashMap<u64, Pending>> = RefCell::new(HashMap::new());
}
static NEXT: AtomicU64 = AtomicU64::new(1);
const KEEP: Duration = Duration::from_secs(60);
/// MapKit offers up to 15; the list shows a few.
const MAX: usize = 6;

fn start(query: &str, reply: Reply) {
    let Some(mtm) = MainThreadMarker::new() else {
        let _ = reply.send(Err(anyhow!("Apple Maps must be asked on the main thread")));
        return;
    };
    // Answered ones go now: never from inside their own callback.
    PENDING.with_borrow_mut(|p| p.retain(|_, q| q.started.elapsed() < KEEP && q._delegate.ivars().reply.borrow().is_some()));
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let delegate = Delegate::new(mtm, reply);
    // SAFETY: a plain NSObject subclass from MapKit; the delegate outlives it in PENDING.
    let completer = unsafe { MKLocalSearchCompleter::new() };
    unsafe {
        completer.setResultTypes(MKLocalSearchCompleterResultType::Address | MKLocalSearchCompleterResultType::PointOfInterest);
        completer.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        completer.setQueryFragment(&NSString::from_str(query));
    }
    PENDING.with_borrow_mut(|p| p.insert(id, Pending { _completer: completer, _delegate: delegate, started: Instant::now() }));
}

/// A completion as a suggestion: "Blue Bottle Coffee" / "66 Mint St, San Francisco, CA".
pub fn suggestion(title: &str, subtitle: &str) -> PlaceSuggestion {
    // Apple's addresses have double spaces ("CA  94115").
    let tidy = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let (title, subtitle) = (&tidy(title), &tidy(subtitle));
    PlaceSuggestion {
        text: if subtitle.is_empty() { title.to_string() } else { format!("{title}, {subtitle}") },
        label: title.to_string(),
        detail: (!subtitle.is_empty()).then(|| subtitle.to_string()),
        source: PlaceSource::AppleMaps,
    }
}

struct Ivars {
    reply: RefCell<Option<Reply>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and Delegate has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SatchelPlaceCompleterDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl MKLocalSearchCompleterDelegate for Delegate {
        #[unsafe(method(completerDidUpdateResults:))]
        fn completer_did_update_results(&self, completer: &MKLocalSearchCompleter) {
            // SAFETY: called by MapKit on the main thread with a live completer.
            let results = unsafe { completer.results() };
            let found = results.iter().take(MAX).map(|c| unsafe { suggestion(&c.title().to_string(), &c.subtitle().to_string()) }).collect();
            self.answer(Ok(found));
        }

        #[unsafe(method(completer:didFailWithError:))]
        fn completer_did_fail(&self, _completer: &MKLocalSearchCompleter, error: &NSError) {
            self.answer(Err(anyhow!("Apple Maps: {}", error.localizedDescription())));
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, reply: Reply) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { reply: RefCell::new(Some(reply)) });
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }

    /// The first answer wins; MapKit may refine its results later.
    fn answer(&self, r: Result<Vec<PlaceSuggestion>>) {
        if let Some(tx) = self.ivars().reply.borrow_mut().take() {
            let _ = tx.send(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_become_suggestions() {
        let s = suggestion("Blue Bottle Coffee", "66 Mint St, San Francisco, CA, United States");
        assert_eq!(s.text, "Blue Bottle Coffee, 66 Mint St, San Francisco, CA, United States");
        assert_eq!(s.detail.as_deref(), Some("66 Mint St, San Francisco, CA, United States"));
        assert_eq!(suggestion("Paris", " ").text, "Paris");
        assert_eq!(suggestion("Cafe", "Main St, CA  94115").text, "Cafe, Main St, CA 94115");
    }
}
