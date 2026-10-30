//! Microsoft Graph: Outlook / Microsoft 365 mail, calendar and contacts.
//!
//! Uses immutable ids so messages keep their id across folder moves. Calendar
//! events are synced as concrete instances (calendarView delta), so recurring
//! series appear as their occurrences.

pub mod calendar;
pub mod contacts;
pub mod mail;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone, Utc};
use futures::StreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CalendarOp, CalendarProvider, CredentialHandle, MailProvider, RemoteCalendar, RemoteMailbox, ResolvedOp, SyncCtx};
use satchel_core::store::{MessageRecord, MessageRef};
use serde_json::{Value, json};
use tracing::warn;

use crate::oauth::{ApiClient, OAuthApp, TokenSource};

const ME: &str = "https://graph.microsoft.com/v1.0/me";

/// Whether a URL from Graph (a nextLink or deltaLink) or a cursor saved from
/// one points at Graph itself, so it may get the bearer token.
pub(crate) fn is_graph(url: &str) -> bool {
    url.starts_with("https://graph.microsoft.com/")
}

/// A next or delta link from a Graph response, refused unless it's on Graph.
pub(crate) fn graph_link(url: &str) -> Result<String> {
    if !is_graph(url) {
        anyhow::bail!("Microsoft Graph sent a link to another host: {}", url.chars().take(100).collect::<String>());
    }
    Ok(url.to_string())
}


pub fn api_client(creds: CredentialHandle) -> Result<ApiClient> {
    Ok(ApiClient { http: crate::net::http_client(false)?, tokens: TokenSource::new(OAuthApp::microsoft(), creds)? })
}

pub(crate) fn with_prefs(rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    rb.header("Prefer", "IdType=\"ImmutableId\", outlook.timezone=\"UTC\", odata.maxpagesize=200")
}

pub(crate) fn addr(v: &Value) -> Option<Address> {
    let e = &v["emailAddress"];
    Some(Address { email: e["address"].as_str()?.to_string(), name: e["name"].as_str().filter(|s| !s.is_empty()).map(String::from) })
}

pub(crate) fn addrs(v: &Value) -> Vec<Address> {
    v.as_array().map(|a| a.iter().filter_map(addr).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_graph_links_get_the_token() {
        assert!(super::graph_link("https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=x").is_ok());
        for bad in ["https://graph.microsoft.com.evil.test/v1.0/", "http://graph.microsoft.com/v1.0/", "https://evil.test/https://graph.microsoft.com/", "https://graph.microsoft.com@evil.test/"] {
            assert!(super::graph_link(bad).is_err(), "{bad}");
        }
    }
}
