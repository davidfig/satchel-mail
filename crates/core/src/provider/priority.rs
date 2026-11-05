//! The user goes first (0037): while the user waits on the server (opening
//! a message, downloading an attachment), the account's background sync
//! pauses at its next request instead of competing for the connection and
//! the provider's rate limit.
//!
//! The lane travels with the task (a tokio task-local), so provider code
//! needs no extra arguments: [`yield_to_user`] before each background
//! request, and [`is_user`] to keep the user's own retries short.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

/// The longest background work waits for the user in one go, in case a
/// user request hangs; user requests have their own, shorter timeouts.
const MAX_PAUSE: Duration = Duration::from_secs(60);

/// One account's user requests in flight.
#[derive(Default)]
pub struct Gate {
    users: AtomicUsize,
    idle: Notify,
}

impl Gate {
    /// Whether a user request is waiting on this account's server.
    pub fn busy(&self) -> bool {
        self.users.load(Ordering::SeqCst) > 0
    }

    /// Wait until no user request is in flight (at most [`MAX_PAUSE`]).
    pub async fn wait_for_users(&self) {
        let _ = tokio::time::timeout(MAX_PAUSE, async {
            loop {
                let idle = self.idle.notified();
                tokio::pin!(idle);
                idle.as_mut().enable();
                if !self.busy() {
                    return;
                }
                idle.await;
            }
        })
        .await;
    }
}

/// Counts a user request for as long as it lives, even if it is cancelled.
struct UserGuard(Arc<Gate>);

impl Drop for UserGuard {
    fn drop(&mut self) {
        if self.0.users.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

#[derive(Clone)]
enum Lane {
    User,
    Background(Arc<Gate>),
}

tokio::task_local! {
    static LANE: Lane;
}

/// Run `f` as a user request: background work on the account pauses until it ends.
pub async fn as_user<F: Future>(gate: Arc<Gate>, f: F) -> F::Output {
    gate.users.fetch_add(1, Ordering::SeqCst);
    let _guard = UserGuard(gate);
    LANE.scope(Lane::User, f).await
}

/// Run `f` as background work that gives way to the account's user requests.
pub async fn in_background<F: Future>(gate: Arc<Gate>, f: F) -> F::Output {
    LANE.scope(Lane::Background(gate), f).await
}

/// In background work, wait while the user is waiting on the same account.
/// Anywhere else, return at once.
pub async fn yield_to_user() {
    let gate = LANE.try_with(|l| match l {
        Lane::Background(g) => Some(g.clone()),
        Lane::User => None,
    });
    if let Ok(Some(g)) = gate
        && g.busy()
    {
        g.wait_for_users().await;
    }
}

/// Whether the current task is a user request, which should fail fast
/// rather than sit out a long rate-limit backoff.
pub fn is_user() -> bool {
    LANE.try_with(|l| matches!(l, Lane::User)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn background_waits_for_the_user() {
        let gate = Arc::new(Gate::default());
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let user = tokio::spawn({
            let gate = gate.clone();
            async move {
                as_user(gate, async {
                    assert!(is_user());
                    rx.await.unwrap()
                })
                .await
            }
        });
        while !gate.busy() {
            tokio::task::yield_now().await;
        }
        let bg = tokio::spawn(in_background(gate.clone(), async { yield_to_user().await }));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !bg.is_finished(),
            "background waits while the user request runs"
        );
        tx.send(()).unwrap();
        user.await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), bg)
            .await
            .expect("background resumes")
            .unwrap();
        assert!(!is_user());
    }

    #[tokio::test]
    async fn a_cancelled_user_request_releases_the_gate() {
        let gate = Arc::new(Gate::default());
        let r = tokio::time::timeout(
            Duration::from_millis(10),
            as_user(gate.clone(), std::future::pending::<()>()),
        )
        .await;
        assert!(r.is_err());
        assert!(!gate.busy());
        tokio::time::timeout(Duration::from_secs(1), in_background(gate, yield_to_user()))
            .await
            .expect("no wait");
    }
}
