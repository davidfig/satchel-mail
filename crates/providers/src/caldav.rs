//! CalDAV calendar provider (RFC 4791, sync via RFC 6578 with an ETag fallback).

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::Event as XEvent;
use reqwest::Method;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CalendarOp, CalendarProvider, CredentialHandle, RemoteCalendar, SyncCtx};
use tokio::sync::Mutex;
use tracing::{debug, warn};

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
    r.config_mut().trim_text(true);
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
                text.push_str(match name {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
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

pub struct CalDavProvider {
    base: String,
    username: String,
    creds: CredentialHandle,
    http: reqwest::Client,
    home: Mutex<Option<String>>,
    owner_email: String,
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl CalDavProvider {
    pub fn new(base: String, username: String, owner_email: String, allow_invalid: bool, creds: CredentialHandle) -> Result<Self> {
        Ok(Self { base, username, creds, http: crate::net::http_client(allow_invalid)?, home: Mutex::new(None), owner_email })
    }

    fn url(&self, href: &str) -> Result<String> {
        Ok(url::Url::parse(&self.base)?.join(href)?.to_string())
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        Ok(match self.creds.get()? {
            Credential::Password { password } => rb.basic_auth(&self.username, Some(password)),
            Credential::OAuth { access_token, .. } => rb.bearer_auth(access_token),
        })
    }

    async fn dav(&self, method: &str, url: &str, depth: Option<&str>, body: Option<String>, extra: &[(&str, &str)]) -> Result<(u16, reqwest::header::HeaderMap, String)> {
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
        let resp = self.auth(rb)?.send().await.with_context(|| format!("CalDAV {method} {url}"))?;
        let st = resp.status().as_u16();
        let headers = resp.headers().clone();
        let text = resp.text().await.unwrap_or_default();
        Ok((st, headers, text))
    }

    async fn propfind(&self, url: &str, depth: &str, props: &str) -> Result<Vec<DavResponse>> {
        let body = format!(r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:ic="http://apple.com/ns/ical/"><d:prop>{props}</d:prop></d:propfind>"#);
        let mut target = url.to_string();
        for _ in 0..5 {
            let (st, headers, text) = self.dav("PROPFIND", &target, Some(depth), Some(body.clone()), &[]).await?;
            if (300..400).contains(&st) {
                let loc = headers.get("location").and_then(|v| v.to_str().ok()).context("redirect without location")?;
                target = url::Url::parse(&target)?.join(loc)?.to_string();
                continue;
            }
            if st == 401 {
                return Err(anyhow::Error::new(ApiError::new(ErrorCode::Auth, "CalDAV login failed")));
            }
            if st != 207 {
                return Err(anyhow::Error::new(ApiError::new(ErrorCode::Network, format!("CalDAV PROPFIND {target}: HTTP {st}"))));
            }
            return Ok(parse_multistatus(&text)?.0);
        }
        Err(anyhow!("too many redirects"))
    }

    /// current-user-principal -> calendar-home-set.
    async fn home(&self) -> Result<String> {
        if let Some(h) = self.home.lock().await.clone() {
            return Ok(h);
        }
        let start = self.base.clone();
        let r = self.propfind(&start, "0", "<d:current-user-principal/><c:calendar-home-set/>").await?;
        let mut home = r.iter().find_map(|x| x.props.get("calendar-home-set").filter(|s| !s.is_empty()).cloned());
        if home.is_none() {
            let principal = r
                .iter()
                .find_map(|x| x.props.get("current-user-principal").filter(|s| !s.is_empty()).cloned())
                .context("server did not report a CalDAV principal")?;
            let r2 = self.propfind(&self.url(&principal)?, "0", "<c:calendar-home-set/>").await?;
            home = r2.iter().find_map(|x| x.props.get("calendar-home-set").filter(|s| !s.is_empty()).cloned());
        }
        let home = self.url(&home.context("server did not report a calendar home")?)?;
        *self.home.lock().await = Some(home.clone());
        Ok(home)
    }

    async fn calendars(&self) -> Result<Vec<(RemoteCalendar, Option<String>)>> {
        let home = self.home().await?;
        let r = self
            .propfind(&home, "1", "<d:resourcetype/><d:displayname/><ic:calendar-color/><d:current-user-privilege-set/><c:supported-calendar-component-set/><d:sync-token/><cs:getctag/>")
            .await?;
        let mut out = vec![];
        for x in r {
            if !x.resource_types.iter().any(|t| t == "calendar") {
                continue;
            }
            if !x.components.is_empty() && !x.components.iter().any(|c| c.eq_ignore_ascii_case("VEVENT")) {
                continue;
            }
            let read_only = !x.privileges.is_empty() && !x.privileges.iter().any(|p| p == "write" || p == "write-content" || p == "all");
            let name = x.props.get("displayname").filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| {
                x.href.trim_end_matches('/').rsplit('/').next().unwrap_or("Calendar").to_string()
            });
            let color = x.props.get("calendar-color").filter(|c| c.starts_with('#')).map(|c| c.chars().take(7).collect());
            out.push((RemoteCalendar { remote_id: x.href.clone(), name, color, read_only }, x.props.get("sync-token").cloned()));
        }
        Ok(out)
    }

    async fn multiget(&self, cal_url: &str, hrefs: &[String]) -> Result<Vec<(String, Option<String>, String)>> {
        let mut out = vec![];
        for chunk in hrefs.chunks(100) {
            let refs: String = chunk.iter().map(|h| format!("<d:href>{}</d:href>", xml_escape(h))).collect();
            let body = format!(r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop>{refs}</c:calendar-multiget>"#);
            let (st, _, text) = self.dav("REPORT", cal_url, Some("1"), Some(body), &[]).await?;
            if st != 207 {
                return Err(anyhow!("calendar-multiget HTTP {st}"));
            }
            for x in parse_multistatus(&text)?.0 {
                if let Some(data) = x.props.get("calendar-data").filter(|d| !d.is_empty()) {
                    out.push((x.href.clone(), x.props.get("getetag").cloned(), data.clone()));
                }
            }
        }
        Ok(out)
    }

    async fn sync_calendar(&self, ctx: &SyncCtx, local_id: &str, cal_href: &str, token: Option<String>) -> Result<()> {
        let cal_url = self.url(cal_href)?;
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:sync-collection xmlns:d="DAV:"><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
            xml_escape(token.as_deref().unwrap_or(""))
        );
        let (st, _, text) = self.dav("REPORT", &cal_url, None, Some(body), &[]).await?;
        let known = ctx.store.event_remote_ids(local_id)?;
        let (changed, deleted, new_token) = if st == 207 {
            let (resps, tok) = parse_multistatus(&text)?;
            let mut changed = vec![];
            let mut deleted = vec![];
            for x in resps {
                if x.href.trim_end_matches('/') == cal_href.trim_end_matches('/') {
                    continue;
                }
                if x.status == Some(404) {
                    deleted.push(x.href);
                } else {
                    let etag = x.props.get("getetag").cloned();
                    if known.get(&x.href).is_none_or(|(_, e)| *e != etag) {
                        changed.push(x.href);
                    }
                }
            }
            (changed, deleted, tok)
        } else {
            // No sync-collection (or token expired): compare ETags.
            debug!("sync-collection unavailable ({st}); comparing etags");
            let r = self.propfind(&cal_url, "1", "<d:getetag/><d:resourcetype/>").await?;
            let mut seen = std::collections::HashSet::new();
            let mut changed = vec![];
            for x in r.iter().filter(|x| !x.resource_types.iter().any(|t| t == "collection")) {
                seen.insert(x.href.clone());
                let etag = x.props.get("getetag").cloned();
                if known.get(&x.href).is_none_or(|(_, e)| *e != etag) {
                    changed.push(x.href.clone());
                }
            }
            let deleted = known.keys().filter(|k| !seen.contains(*k)).cloned().collect();
            (changed, deleted, None)
        };
        for d in &deleted {
            ctx.delete_event_remote(local_id, d)?;
        }
        if !changed.is_empty() {
            for (href, etag, data) in self.multiget(&cal_url, &changed).await? {
                ctx.put_event(local_id, &href, etag.as_deref(), &data)?;
            }
        }
        if let Some(t) = new_token {
            ctx.store.set_calendar_token(local_id, Some(&t))?;
        }
        Ok(())
    }
}

#[async_trait]
impl CalendarProvider for CalDavProvider {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let cals = self.calendars().await?;
        let map = ctx.set_calendars(&cals.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>())?;
        for (c, _) in &cals {
            let Some(local) = map.get(&c.remote_id) else { continue };
            let token = ctx.store.get_calendar(local)?.and_then(|s| s.sync_token);
            if let Err(e) = self.sync_calendar(ctx, local, &c.remote_id, token.clone()).await {
                if token.is_some() {
                    // Token may be invalid; start over once.
                    warn!(calendar = %c.name, "sync with token failed ({e:#}); full resync");
                    ctx.store.set_calendar_token(local, None)?;
                    self.sync_calendar(ctx, local, &c.remote_id, None).await?;
                } else {
                    return Err(e);
                }
            }
        }
        let _ = &self.owner_email;
        Ok(())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &CalendarOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            CalendarOp::Put { calendar_remote_id, event_remote_id, etag, uid, ics } => {
                let href = match event_remote_id {
                    Some(h) => h.clone(),
                    None => {
                        let safe: String = uid.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
                        format!("{}/{}.ics", calendar_remote_id.trim_end_matches('/'), safe)
                    }
                };
                let url = self.url(&href)?;
                let mut rb = self.http.put(&url).header("Content-Type", "text/calendar; charset=utf-8").body(ics.clone());
                rb = match (event_remote_id, etag) {
                    (None, _) => rb.header("If-None-Match", "*"),
                    (Some(_), Some(e)) => rb.header("If-Match", e.as_str()),
                    _ => rb,
                };
                let resp = self.auth(rb)?.send().await?;
                let st = resp.status();
                if st.as_u16() == 412 {
                    return Err(anyhow::Error::new(ApiError::new(ErrorCode::Conflict, "the event changed on the server; refresh and try again")));
                }
                if !st.is_success() {
                    return Err(crate::net::http_error(st, &resp.text().await.unwrap_or_default(), "CalDAV PUT"));
                }
                out.event_etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
                out.event_remote_id = Some(href);
            }
            CalendarOp::Delete { event_remote_id, etag, .. } => {
                let url = self.url(event_remote_id)?;
                let mut rb = self.http.delete(&url);
                if let Some(e) = etag {
                    rb = rb.header("If-Match", e.as_str());
                }
                let resp = self.auth(rb)?.send().await?;
                let st = resp.status();
                if !(st.is_success() || st.as_u16() == 404) {
                    return Err(crate::net::http_error(st, &resp.text().await.unwrap_or_default(), "CalDAV DELETE"));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calendar_listing() {
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:" xmlns:A="urn:ietf:params:xml:ns:caldav" xmlns:I="http://apple.com/ns/ical/">
          <D:response><D:href>/dav/cal/a%40x/default/</D:href><D:propstat><D:prop>
            <D:resourcetype><D:collection/><A:calendar/></D:resourcetype><D:displayname>Default</D:displayname>
            <I:calendar-color>#ff0000ff</I:calendar-color>
            <D:current-user-privilege-set><D:privilege><D:read/></D:privilege><D:privilege><D:write/></D:privilege></D:current-user-privilege-set>
            <A:supported-calendar-component-set><A:comp name="VEVENT"/><A:comp name="VTODO"/></A:supported-calendar-component-set>
          </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
          <D:propstat><D:prop><D:getctag/></D:prop><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat></D:response>
          <D:response><D:href>/dav/cal/a%40x/gone.ics</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>
          <D:sync-token>tok-1</D:sync-token></D:multistatus>"#;
        let (r, tok) = parse_multistatus(xml).unwrap();
        assert_eq!(tok.as_deref(), Some("tok-1"));
        assert_eq!(r[0].href, "/dav/cal/a%40x/default/");
        assert!(r[0].resource_types.contains(&"calendar".to_string()));
        assert_eq!(r[0].props["displayname"], "Default");
        assert!(r[0].privileges.contains(&"write".to_string()));
        assert_eq!(r[0].components, vec!["VEVENT", "VTODO"]);
        assert!(!r[0].props.contains_key("getctag"));
        assert_eq!(r[1].status, Some(404));
    }

    #[test]
    fn parses_principal_href() {
        let xml = r#"<D:multistatus xmlns:D="DAV:" xmlns:A="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/dav/cal/</D:href><D:propstat><D:prop><D:current-user-principal><D:href>/dav/pal/a%40x/</D:href></D:current-user-principal><A:calendar-home-set><D:href>/dav/cal/a%40x/</D:href></A:calendar-home-set></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        let (r, _) = parse_multistatus(xml).unwrap();
        assert_eq!(r[0].props["current-user-principal"], "/dav/pal/a%40x/");
        assert_eq!(r[0].props["calendar-home-set"], "/dav/cal/a%40x/");
    }
}
