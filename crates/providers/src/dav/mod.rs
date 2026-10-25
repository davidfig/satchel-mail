//! WebDAV plumbing shared by CalDAV and CardDAV: requests with the account's
//! credentials, PROPFIND with redirects, and a multistatus parser.

pub mod caldav;
pub mod carddav;

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use quick_xml::Reader;
use quick_xml::events::Event as XEvent;
use reqwest::Method;
use satchel_api::*;
use satchel_core::provider::CredentialHandle;

/// One `<response>` of a multistatus.
#[derive(Debug, Default, Clone)]
pub struct DavResponse {
    pub href: String,
    /// Status of the whole response (e.g. 404 for deleted members in sync reports).
    pub status: Option<u16>,
    /// Local property name -> text content (or child element names for containers).
    pub props: HashMap<String, String>,
    /// Child element local names under resourcetype.
    pub resource_types: Vec<String>,
    /// Privileges under current-user-privilege-set.
    pub privileges: Vec<String>,
    /// Components under supported-calendar-component-set.
    pub components: Vec<String>,
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

/// Namespace-agnostic multistatus parser.
pub fn parse_multistatus(xml: &str) -> Result<(Vec<DavResponse>, Option<String>)> {
    let mut r = Reader::from_str(xml);
    // Keep whitespace: card and calendar data are line-based (values are trimmed below).
    r.config_mut().trim_text(false);
    let mut out = vec![];
    let mut sync_token = None;
    let mut cur: Option<DavResponse> = None;
    let mut stack: Vec<String> = vec![];
    let mut text = String::new();
    // Status applies to the propstat it is in; only propstats with 2xx count.
    let mut propstat_props: HashMap<String, String> = HashMap::new();
    let mut propstat_rt: Vec<String> = vec![];
    let mut propstat_priv: Vec<String> = vec![];
    let mut propstat_comp: Vec<String> = vec![];
    let mut propstat_status: Option<u16> = None;
    loop {
        match r.read_event()? {
            XEvent::Start(e) => {
                let n = local(e.name().as_ref());
                if n == "response" {
                    cur = Some(DavResponse::default());
                }
                if n == "propstat" {
                    propstat_props.clear();
                    propstat_rt.clear();
                    propstat_priv.clear();
                    propstat_comp.clear();
                    propstat_status = None;
                }
                stack.push(n);
                text.clear();
            }
            XEvent::Empty(e) => {
                let n = local(e.name().as_ref());
                let parent = stack.last().map(|s| s.as_str()).unwrap_or("");
                if parent == "resourcetype" {
                    propstat_rt.push(n.clone());
                } else if stack.iter().any(|s| s == "privilege") {
                    propstat_priv.push(n.clone());
                } else if parent == "supported-calendar-component-set" && n == "comp" {
                    if let Some(a) = e.attributes().flatten().find(|a| local(a.key.as_ref()) == "name") {
                        propstat_comp.push(a.value.to_string());
                    }
                } else if parent == "prop" {
                    propstat_props.insert(n, String::new());
                }
            }
            XEvent::Text(t) => {
                text.push_str(&t);
            }
            XEvent::CData(t) => {
                text.push_str(&t);
            }
            XEvent::GeneralRef(g) => {
                let name: &str = &g;
                match name {
                    "amp" => text.push('&'),
                    "lt" => text.push('<'),
                    "gt" => text.push('>'),
                    "quot" => text.push('"'),
                    "apos" => text.push('\''),
                    n => {
                        // Character references: &#13; &#x1F600;
                        let code = n.strip_prefix("#x").or(n.strip_prefix("#X")).map(|h| u32::from_str_radix(h, 16)).or(n.strip_prefix('#').map(|d| d.parse::<u32>()));
                        if let Some(c) = code.and_then(|r| r.ok()).and_then(char::from_u32) {
                            text.push(c);
                        }
                    }
                }
            }
            XEvent::End(e) => {
                let n = local(e.name().as_ref());
                stack.pop();
                let parent = stack.last().cloned().unwrap_or_default();
                match n.as_str() {
                    "href" if parent == "response" => {
                        if let Some(c) = cur.as_mut() {
                            // Keep the server's exact encoding: hrefs are echoed back in later requests.
                            c.href = text.trim().to_string();
                        }
                    }
                    "status" if parent == "response" => {
                        if let Some(c) = cur.as_mut() {
                            c.status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok());
                        }
                    }
                    "status" if parent == "propstat" => {
                        propstat_status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok());
                    }
                    "propstat" => {
                        if let Some(c) = cur.as_mut()
                            && propstat_status.is_none_or(|s| (200..300).contains(&s)) {
                                c.props.extend(propstat_props.drain());
                                c.resource_types.append(&mut propstat_rt);
                                c.privileges.append(&mut propstat_priv);
                                c.components.append(&mut propstat_comp);
                            }
                    }
                    "response" => {
                        if let Some(c) = cur.take() {
                            out.push(c);
                        }
                    }
                    "sync-token" if parent == "multistatus" => sync_token = Some(text.trim().to_string()),
                    _ => {
                        if parent == "prop" {
                            // Don't clobber a value captured from a nested <href>.
                            let v = text.trim().to_string();
                            if !v.is_empty() || !propstat_props.contains_key(&n) {
                                propstat_props.insert(n, v);
                            }
                        } else if parent == "resourcetype" {
                            propstat_rt.push(n);
                        } else if stack.iter().any(|s| s == "privilege") {
                            propstat_priv.push(n);
                        } else if stack.iter().rev().nth(1).is_some_and(|g| g == "prop") && n == "href" {
                            // e.g. <current-user-principal><href>…</href></current-user-principal>
                            let p = parent.clone();
                            propstat_props.insert(p, text.trim().to_string());
                        }
                    }
                }
                text.clear();
            }
            XEvent::Eof => break,
            _ => {}
        }
    }
    Ok((out, sync_token))
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// An authenticated WebDAV client rooted at the account's DAV URL.
pub struct DavClient {
    pub base: String,
    username: String,
    creds: CredentialHandle,
    pub http: reqwest::Client,
    /// "CalDAV" or "CardDAV", for messages.
    label: &'static str,
}

impl DavClient {
    pub fn new(base: String, username: String, allow_invalid: bool, creds: CredentialHandle, label: &'static str) -> Result<Self> {
        Ok(Self { base, username, creds, http: crate::net::http_client(allow_invalid)?, label })
    }

    pub fn url(&self, href: &str) -> Result<String> {
        Ok(url::Url::parse(&self.base)?.join(href)?.to_string())
    }

    pub fn auth(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        Ok(match self.creds.get()? {
            Credential::Password { password } => rb.basic_auth(&self.username, Some(password)),
            Credential::OAuth { access_token, .. } => rb.bearer_auth(access_token),
        })
    }

    pub async fn dav(&self, method: &str, url: &str, depth: Option<&str>, body: Option<String>, extra: &[(&str, &str)]) -> Result<(u16, reqwest::header::HeaderMap, String)> {
        let mut rb = self.http.request(Method::from_bytes(method.as_bytes())?, url);
        if let Some(d) = depth {
            rb = rb.header("Depth", d);
        }
        if let Some(b) = body {
            rb = rb.header("Content-Type", "application/xml; charset=utf-8").body(b);
        }
        for (k, v) in extra {
            rb = rb.header(*k, *v);
        }
        let label = self.label;
        let resp = self.auth(rb)?.send().await.with_context(|| format!("{label} {method} {url}"))?;
        let st = resp.status().as_u16();
        let headers = resp.headers().clone();
        let text = resp.text().await.unwrap_or_default();
        Ok((st, headers, text))
    }

    pub async fn propfind(&self, url: &str, depth: &str, props: &str) -> Result<Vec<DavResponse>> {
        let body = format!(r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:cs="http://calendarserver.org/ns/" xmlns:ic="http://apple.com/ns/ical/"><d:prop>{props}</d:prop></d:propfind>"#);
        let label = self.label;
        let mut target = url.to_string();
        for _ in 0..5 {
            let (st, headers, text) = self.dav("PROPFIND", &target, Some(depth), Some(body.clone()), &[]).await?;
            if (300..400).contains(&st) {
                let loc = headers.get("location").and_then(|v| v.to_str().ok()).context("redirect without location")?;
                target = url::Url::parse(&target)?.join(loc)?.to_string();
                continue;
            }
            if st == 401 {
                return Err(anyhow::Error::new(ApiError::new(ErrorCode::Auth, format!("{label} login failed"))));
            }
            if st != 207 {
                return Err(anyhow::Error::new(ApiError::new(ErrorCode::Network, format!("{label} PROPFIND {target}: HTTP {st}"))));
            }
            return Ok(parse_multistatus(&text)?.0);
        }
        Err(anyhow!("too many redirects"))
    }

    /// Find a home-set property (`calendar-home-set`, `addressbook-home-set`)
    /// on the base URL or, failing that, on the current user's principal.
    pub async fn home(&self, prop_xml: &str, prop: &str) -> Result<Option<String>> {
        let r = self.propfind(&self.base.clone(), "0", &format!("<d:current-user-principal/>{prop_xml}")).await?;
        let mut home = r.iter().find_map(|x| x.props.get(prop).filter(|s| !s.is_empty()).cloned());
        if home.is_none()
            && let Some(principal) = r.iter().find_map(|x| x.props.get("current-user-principal").filter(|s| !s.is_empty()).cloned())
        {
            let r2 = self.propfind(&self.url(&principal)?, "0", prop_xml).await?;
            home = r2.iter().find_map(|x| x.props.get(prop).filter(|s| !s.is_empty()).cloned());
        }
        home.map(|h| self.url(&h)).transpose()
    }
}
