//! Protocol adapters. [`Factory`] maps an account's config to provider
//! instances for the core.

pub mod caldav;
pub mod gmail;
pub mod graph;
pub mod imap;
pub mod jmap;
pub mod net;
pub mod oauth;

use std::sync::Arc;

use anyhow::{Result, bail};
use satchel_api::*;
use satchel_core::provider::{CalendarProvider, CredentialHandle, OAuthDone, ProviderFactory, Providers};

fn caldav_for(account: &Account, url: Option<&str>, username: &str, insecure: bool, creds: &CredentialHandle) -> Result<Option<Arc<dyn CalendarProvider>>> {
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Ok(Some(Arc::new(caldav::CalDavProvider::new(u.to_string(), username.to_string(), account.email.clone(), insecure, creds.clone())?))),
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

impl ProviderFactory for Factory {
    fn create(&self, account: &Account, creds: CredentialHandle) -> Result<Providers> {
        match &account.config {
            AccountConfig::Imap { username, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, allow_invalid_certs, caldav_url, .. } => {
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
                Ok(Providers { mail: Some(Arc::new(mail)), calendar })
            }
            AccountConfig::Jmap { username, session_url, caldav_url, allow_invalid_certs, .. } => {
                let mail = jmap::JmapProvider::new(session_url.clone(), username.clone(), *allow_invalid_certs, creds.clone())?;
                let calendar = caldav_for(account, caldav_url.as_deref(), username, *allow_invalid_certs, &creds)?;
                Ok(Providers { mail: Some(Arc::new(mail)), calendar })
            }
            AccountConfig::Gmail { .. } => Ok(Providers {
                mail: Some(Arc::new(gmail::GmailProvider::new(creds.clone())?)),
                calendar: Some(Arc::new(gmail::GoogleCalendarProvider::new(creds, account.email.clone())?)),
            }),
            AccountConfig::Graph { .. } => Ok(Providers {
                mail: Some(Arc::new(graph::GraphMailProvider::new(creds.clone())?)),
                calendar: Some(Arc::new(graph::GraphCalendarProvider::new(creds, account.email.clone())?)),
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
}
