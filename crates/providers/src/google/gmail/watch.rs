//! Gmail's new-mail signal: IMAP IDLE on the inbox (0060).
//!
//! The Gmail API's own push needs a Cloud Pub/Sub topic, which a local app
//! doesn't have. This connection only says when to look: it opens the inbox
//! read-only and all mail still comes through the API. If it can't connect,
//! the account simply polls.

use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use async_imap::Authenticator;
use satchel_api::Security;
use tracing::debug;

use super::GmailProvider;
use crate::imap::{Client, Session, idle_until_change, open_client};

const IMAP_HOST: &str = "imap.gmail.com";
const IMAP_PORT: u16 = 993;
/// How long getting a token, connecting and opening the inbox may take.
const SETUP: Duration = Duration::from_secs(30);

/// SASL XOAUTH2. When Google refuses the token it sends an error challenge,
/// which must be answered with an empty response.
struct XOAuth2(Option<String>);

impl Authenticator for XOAuth2 {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> String {
        self.0.take().unwrap_or_default()
    }
}

/// Sign in and open the inbox read-only. Returns its message count.
pub(super) async fn open_inbox(client: Client, user: &str, token: &str) -> Result<(Session, u32)> {
    let sasl = XOAuth2(Some(format!("user={user}\x01auth=Bearer {token}\x01\x01")));
    let mut s = client.authenticate("XOAUTH2", sasl).await.map_err(|(e, _)| anyhow!("IMAP sign-in failed: {e}"))?;
    let inbox = s.examine("INBOX").await.context("open inbox")?;
    Ok((s, inbox.exists))
}

/// Wait for a change in the open inbox. A count different from the last wait's
/// means mail came while nothing was listening, so there's no need to wait.
/// Mail that wakes a wait also changes the count, so each arrival gets a second
/// pass, which picks up anything the API hadn't shown the first one.
pub(super) async fn wait_on_inbox(mut s: Session, exists: u32, last: &Mutex<Option<u32>>, max_wait: Duration) -> Result<bool> {
    let previous = last.lock().unwrap().replace(exists);
    if previous.is_some_and(|p| p != exists) {
        let _ = tokio::time::timeout(Duration::from_secs(10), s.logout()).await;
        return Ok(true);
    }
    idle_until_change(s, max_wait).await
}

/// Run a watch; if it fails, sleep out the rest of `max_wait` as a plain poll.
/// A failure here never reaches the account's status: the API sync reports
/// any real sign-in problem.
pub(super) async fn or_poll(watch: impl Future<Output = Result<bool>>, max_wait: Duration) {
    let started = Instant::now();
    if let Err(e) = watch.await {
        debug!("Gmail IDLE unavailable, polling: {e:#}");
        tokio::time::sleep(max_wait.saturating_sub(started.elapsed())).await;
    }
}

impl GmailProvider {
    /// Wait for the inbox to change, up to `max_wait`.
    pub(super) async fn wait_for_inbox(&self, email: &str, max_wait: Duration) -> Result<bool> {
        let started = Instant::now();
        let setup = async {
            let token = self.api.tokens.token().await?;
            let client = open_client(IMAP_HOST, IMAP_PORT, Security::Tls, false, None).await?;
            open_inbox(client, email, &token).await
        };
        let (session, exists) = tokio::time::timeout(SETUP, setup).await.map_err(|_| anyhow!("connecting to Gmail IMAP timed out"))??;
        wait_on_inbox(session, exists, &self.inbox_exists, max_wait.saturating_sub(started.elapsed())).await
    }
}
