//! Google: Gmail, Google Calendar and Google Contacts, one OAuth sign-in.

pub mod calendar;
pub mod contacts;
pub mod gmail;

use anyhow::Result;
use satchel_core::provider::CredentialHandle;

use crate::oauth::{ApiClient, OAuthApp, TokenSource};

pub fn api_client(creds: CredentialHandle) -> Result<ApiClient> {
    Ok(ApiClient { http: crate::net::http_client(false)?, tokens: TokenSource::new(OAuthApp::google(), creds)? })
}
