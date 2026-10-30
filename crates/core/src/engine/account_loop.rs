//! Each account's loop: push queued ops, sync, download bodies, wait for changes.

use super::*;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use satchel_api::*;
use tokio::sync::Notify;
use tracing::{debug, warn};

use crate::provider::{Providers, SyncCtx, priority};

/// Aborts the task when dropped, so aborting the supervisor stops its loop too.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Engine {
    /// Run the account's loop, restarting it after a pause if it panics:
    /// a bug tripped by one message shows as an account error instead of
    /// sync silently stopping mid-pass.
    pub(crate) async fn supervise_account(this: std::sync::Weak<Self>, account_id: String, providers: Arc<Providers>, wake: Arc<Notify>) {
        loop {
            let mut task = AbortOnDrop(tokio::spawn(Self::account_loop(this.clone(), account_id.clone(), providers.clone(), wake.clone())));
            let Err(e) = (&mut task.0).await else { return };
            if !e.is_panic() {
                return;
            }
            let p = e.into_panic();
            let why = p.downcast_ref::<&str>().copied().or_else(|| p.downcast_ref::<String>().map(String::as_str)).unwrap_or("unknown panic");
            let Some(engine) = this.upgrade() else { return };
            warn!(account = %account_id, "sync crashed, restarting in a minute: {why}");
            let last_success = engine.store.get_account(&account_id).ok().flatten().and_then(|a| a.sync.last_success);
            engine.set_status(&account_id, SyncStatus { state: SyncState::Error, last_success, error: Some(format!("Sync stopped unexpectedly ({why}); retrying.")), detail: None });
            drop(engine);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => {},
                _ = wake.notified() => {},
            }
        }
    }

    pub(crate) async fn account_loop(this: std::sync::Weak<Self>, account_id: String, providers: Arc<Providers>, wake: Arc<Notify>) {
        let mut failures: u32 = 0;
        let mut first = true;
        // Address books change rarely; sync them every few minutes, not every poll.
        let mut contacts_synced: Option<std::time::Instant> = None;
        loop {
            let Some(engine) = this.upgrade() else { return };
            let mut ctx = match engine.ctx(&account_id) {
                Ok(c) => c,
                Err(_) => return,
            };
            let started = std::time::Instant::now();
            let last_success = ctx.account.sync.last_success;
            engine.set_status(&account_id, SyncStatus { state: SyncState::Syncing, last_success, error: None, detail: None });

            let mut result: Result<()> = Ok(());
            // Push local changes first so sync doesn't revert optimistic state.
            if let Err(e) = engine.run_ops(&ctx, &providers).await {
                warn!(account = %ctx.account.email, "ops: {e:#}");
            }
            // Background work gives way to the user's own requests on this
            // account (0037); the user's queued changes run between phases.
            let gate = engine.gate(&account_id);
            if let Some(mail) = &providers.mail {
                result = priority::in_background(gate.clone(), mail.sync(&ctx)).await;
                engine.run_due_ops(&ctx, &providers).await;
                if result.is_ok() {
                    // The newest mail is in, so the account counts as synced
                    // even while bodies, older mail and the calendar follow.
                    // Progress reports from here on keep this time.
                    ctx.account.sync.last_success = Some(Utc::now());
                    engine.set_status(&account_id, SyncStatus { state: SyncState::Syncing, last_success: ctx.account.sync.last_success, error: None, detail: None });
                    // The user's sender rules outrank the server's spam filter.
                    // Not while changes are still on their way to the server:
                    // the sync may show messages where they were, and enforcing
                    // would queue the same move again on every poll.
                    if !engine.store.has_pending_ops(&account_id).unwrap_or(true)
                        && let Err(e) = engine.enforce_sender_rules(&account_id)
                    {
                        warn!(account = %ctx.account.email, "sender rules: {e:#}");
                    }
                    engine.fetch_bodies(&ctx, &providers, if first { 300 } else { 100 }).await;
                }
            }
            if let Some(cal) = &providers.calendar {
                engine.run_due_ops(&ctx, &providers).await;
                if let Err(e) = priority::in_background(gate.clone(), cal.sync(&ctx)).await {
                    warn!(account = %ctx.account.email, "calendar sync: {e:#}");
                    if result.is_ok() {
                        result = Err(e);
                    }
                } else if let Err(e) = engine.remove_blocked_invites(&account_id) {
                    warn!(account = %ctx.account.email, "spam invites: {e:#}");
                }
                ctx.calendar_changed();
            }
            if let Some(contacts) = &providers.contacts
                && (contacts_synced.is_none_or(|t| t.elapsed() > CONTACTS_EVERY) || engine.contacts_due.lock().unwrap().remove(&account_id))
            {
                engine.run_due_ops(&ctx, &providers).await;
                priority::in_background(gate.clone(), engine.sync_contacts(&ctx, contacts.as_ref())).await;
                contacts_synced = Some(std::time::Instant::now());
            }
            ctx.messages_changed();
            engine.mirror_wake.notify_one();
            engine.contacts_wake.notify_one();
            match &result {
                Ok(()) => {
                    failures = 0;
                    first = false;
                    debug!(account = %ctx.account.email, "sync done in {:?}", started.elapsed());
                    engine.set_status(&account_id, SyncStatus { state: SyncState::Idle, last_success: Some(Utc::now()), error: None, detail: None });
                }
                Err(e) => {
                    failures += 1;
                    let api = crate::error::to_api(anyhow::anyhow!("{e:#}"));
                    let state = if api.code == ErrorCode::Network { SyncState::Offline } else { SyncState::Error };
                    warn!(account = %ctx.account.email, "sync failed: {e:#}");
                    engine.set_status(&account_id, SyncStatus { state, last_success: ctx.account.sync.last_success, error: Some(format!("{e:#}")), detail: None });
                }
            }
            let poll = Duration::from_secs(engine.settings().poll_interval_secs.max(15) as u64);
            drop(engine);
            if failures == 0 && ctx.has_more() {
                // Older mail left to fetch: carry on after a breath.
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                    _ = wake.notified() => {},
                }
                continue;
            }
            if failures > 0 {
                let backoff = Duration::from_secs((5u64 * 2u64.pow(failures.min(6))).min(600));
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {},
                    _ = wake.notified() => {},
                }
                continue;
            }
            match &providers.mail {
                Some(mail) => {
                    let Some(engine) = this.upgrade() else { return };
                    let ctx = match engine.ctx(&account_id) {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    drop(engine);
                    tokio::select! {
                        r = mail.wait_for_changes(&ctx, poll) => { if let Err(e) = r { debug!("wait: {e:#}"); tokio::time::sleep(Duration::from_secs(5)).await; } },
                        _ = wake.notified() => {},
                    }
                }
                None => {
                    tokio::select! {
                        _ = tokio::time::sleep(poll) => {},
                        _ = wake.notified() => {},
                    }
                }
            }
        }
    }

    /// Push the user's changes queued since the pass began, so they don't
    /// wait for the rest of it.
    pub(crate) async fn run_due_ops(&self, ctx: &SyncCtx, providers: &Providers) {
        if self.store.due_ops(&ctx.account.id).is_ok_and(|o| !o.is_empty())
            && let Err(e) = self.run_ops(ctx, providers).await
        {
            warn!(account = %ctx.account.email, "ops: {e:#}");
        }
    }

    /// One contacts sync. Failures are logged, never shown as an account
    /// error; a sign-in without access to contacts is remembered so Settings
    /// can offer to reconnect.
    pub(crate) async fn sync_contacts(&self, ctx: &SyncCtx, contacts: &dyn crate::provider::ContactsProvider) {
        const KEY: &str = "contacts_scope_missing";
        match contacts.sync(ctx).await {
            Ok(()) => {
                if ctx.cursor(KEY).ok().flatten().is_some() {
                    let _ = self.store.delete_cursor(&ctx.account.id, KEY);
                    self.emit(CoreEvent::AccountsChanged);
                }
            }
            Err(e) if e.downcast_ref::<crate::provider::ContactsScopeMissing>().is_some() => {
                if ctx.cursor(KEY).ok().flatten().is_none() {
                    debug!(account = %ctx.account.email, "contacts: sign-in lacks access to contacts");
                    let _ = ctx.set_cursor(KEY, "1");
                    self.emit(CoreEvent::AccountsChanged);
                }
            }
            Err(e) => warn!(account = %ctx.account.email, "contacts sync: {e:#}"),
        }
        ctx.contacts_changed();
    }
}
