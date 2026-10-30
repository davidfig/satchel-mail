//! Protocol adapters. [`Factory`] maps an account's config to provider
//! instances for the core.

pub mod dav;
pub mod google;
pub mod imap;
pub mod jmap;
pub mod microsoft;
pub mod net;
pub mod oauth;
#[cfg(test)]
mod test_mail;

use std::sync::Arc;

use anyhow::{Result, bail};
use satchel_api::*;
use satchel_core::provider::{CalendarProvider, ContactsProvider, CredentialHandle, OAuthDone, ProviderFactory, Providers};

fn caldav_for(account: &Account, url: Option<&str>, username: &str, insecure: bool, creds: &CredentialHandle) -> Result<Option<Arc<dyn CalendarProvider>>> {
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Ok(Some(Arc::new(dav::caldav::CalDavProvider::new(u.to_string(), username.to_string(), account.email.clone(), insecure, creds.clone())?))),
        None => Ok(None),
    }
}

/// CardDAV from its own URL, else from the CalDAV server (most servers report
/// both homes for the same principal).
fn carddav_for(carddav: Option<&str>, caldav: Option<&str>, username: &str, insecure: bool, creds: &CredentialHandle) -> Result<Option<Arc<dyn ContactsProvider>>> {
    let pick = |u: Option<&str>| u.map(str::trim).filter(|u| !u.is_empty()).map(String::from);
    match pick(carddav).or(pick(caldav)) {
        Some(u) => Ok(Some(Arc::new(dav::carddav::CardDavProvider::new(u, username.to_string(), insecure, creds.clone())?))),
        None => Ok(None),
    }
}

#[derive(Default)]
pub struct Factory {}

impl Factory {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl ProviderFactory for Factory {
    fn create(&self, account: &Account, creds: CredentialHandle) -> Result<Providers> {
        match &account.config {
            AccountConfig::Imap { username, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, allow_invalid_certs, caldav_url, carddav_url, .. } => {
                let mail = imap::ImapProvider::new(
                    imap::ImapSettings {
                        host: imap_host.clone(),
                        port: *imap_port,
                        security: *imap_security,
                        username: username.clone(),
                        smtp_host: smtp_host.clone(),
                        smtp_port: *smtp_port,
                        smtp_security: *smtp_security,
                        allow_invalid_certs: *allow_invalid_certs,
                    },
                    creds.clone(),
                );
                let calendar = caldav_for(account, caldav_url.as_deref(), username, *allow_invalid_certs, &creds)?;
                let contacts = carddav_for(carddav_url.as_deref(), caldav_url.as_deref(), username, *allow_invalid_certs, &creds)?;
                Ok(Providers { mail: Some(Arc::new(mail)), calendar, contacts })
            }
            AccountConfig::Jmap { username, session_url, caldav_url, carddav_url, allow_invalid_certs, .. } => {
                let mail = jmap::JmapProvider::new(session_url.clone(), username.clone(), *allow_invalid_certs, creds.clone())?;
                let calendar = caldav_for(account, caldav_url.as_deref(), username, *allow_invalid_certs, &creds)?;
                let contacts = carddav_for(carddav_url.as_deref(), caldav_url.as_deref(), username, *allow_invalid_certs, &creds)?;
                Ok(Providers { mail: Some(Arc::new(mail)), calendar, contacts })
            }
            AccountConfig::Gmail { .. } => Ok(Providers {
                mail: Some(Arc::new(google::gmail::GmailProvider::new(creds.clone())?)),
                calendar: Some(Arc::new(google::calendar::GoogleCalendarProvider::new(creds.clone(), account.email.clone())?)),
                contacts: Some(Arc::new(google::contacts::GoogleContactsProvider::new(creds)?)),
            }),
            AccountConfig::Graph { .. } => Ok(Providers {
                mail: Some(Arc::new(microsoft::mail::GraphMailProvider::new(creds.clone())?)),
                calendar: Some(Arc::new(microsoft::calendar::GraphCalendarProvider::new(creds.clone(), account.email.clone())?)),
                contacts: Some(Arc::new(microsoft::contacts::GraphContactsProvider::new(creds)?)),
            }),
        }
    }

    fn oauth_start(&self, kind: ProviderKind, done: OAuthDone) -> Result<String> {
        let Some(app) = oauth::OAuthApp::for_kind(kind) else {
            let var = if kind == ProviderKind::Gmail { "SATCHEL_GOOGLE_CLIENT_ID" } else { "SATCHEL_MS_CLIENT_ID" };
            bail!("Sign-in for this provider isn't configured in this build. Set {var} (see docs/OAUTH.md).");
        };
        oauth::start(app, done)
    }

    async fn one_click_unsubscribe(&self, url: &str) -> Result<()> {
        if !url.to_ascii_lowercase().starts_with("https://") {
            bail!("one-click unsubscribe needs an https link");
        }
        // The link comes from the sender: keep it off this computer and the
        // local network, including through DNS and redirects.
        let parsed = url::Url::parse(url)?;
        if net::is_internal_url(&parsed) {
            bail!("one-click unsubscribe link points to a local or private address");
        }
        let http = net::public_http_client()?;
        let r = http
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("List-Unsubscribe=One-Click")
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await?;
        let status = r.status();
        if !status.is_success() {
            let body = net::error_text(r).await;
            return Err(net::http_error(status, &body, "unsubscribe"));
        }
        Ok(())
    }
}
