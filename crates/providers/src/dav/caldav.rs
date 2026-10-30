//! CalDAV calendar provider (RFC 4791, sync via RFC 6578 with an ETag fallback).

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CalendarOp, CalendarProvider, CredentialHandle, RemoteCalendar, SyncCtx};
use tokio::sync::Mutex;
use tracing::{debug, warn};

pub use crate::dav::{DavResponse, parse_multistatus};
use crate::dav::{DavClient, xml_escape};

pub struct CalDavProvider {
    dav: DavClient,
    home: Mutex<Option<String>>,
    owner_email: String,
}

impl CalDavProvider {
    pub fn new(base: String, username: String, owner_email: String, allow_invalid: bool, creds: CredentialHandle) -> Result<Self> {
        Ok(Self { dav: DavClient::new(base, username, allow_invalid, creds, "CalDAV")?, home: Mutex::new(None), owner_email })
    }

    fn url(&self, href: &str) -> Result<String> {
        self.dav.url(href)
    }

    async fn propfind(&self, url: &str, depth: &str, props: &str) -> Result<Vec<DavResponse>> {
        self.dav.propfind(url, depth, props).await
    }

    /// current-user-principal -> calendar-home-set.
    async fn home(&self) -> Result<String> {
        if let Some(h) = self.home.lock().await.clone() {
            return Ok(h);
        }
        let home = self.dav.home("<c:calendar-home-set/>", "calendar-home-set").await?.context("server did not report a calendar home")?;
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
            let (st, _, text) = self.dav.dav("REPORT", cal_url, Some("1"), Some(body), &[]).await?;
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
        let (st, _, text) = self.dav.dav("REPORT", &cal_url, None, Some(body), &[]).await?;
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
                let mut rb = self.dav.http.put(&url).header("Content-Type", "text/calendar; charset=utf-8").body(ics.clone());
                rb = match (event_remote_id, etag) {
                    (None, _) => rb.header("If-None-Match", "*"),
                    (Some(_), Some(e)) => rb.header("If-Match", e.as_str()),
                    _ => rb,
                };
                let resp = self.dav.auth(rb)?.send().await?;
                let st = resp.status();
                if st.as_u16() == 412 {
                    return Err(anyhow::Error::new(ApiError::new(ErrorCode::Conflict, "the event changed on the server; refresh and try again")));
                }
                if !st.is_success() {
                    return Err(crate::net::http_error(st, &crate::net::error_text(resp).await, "CalDAV PUT"));
                }
                out.event_etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
                out.event_remote_id = Some(href);
            }
            CalendarOp::Delete { event_remote_id, etag, notify, .. } => {
                let url = self.url(event_remote_id)?;
                let mut rb = self.dav.http.delete(&url);
                if !notify {
                    // RFC 6638 §8.1: otherwise the server tells the organizer we declined.
                    rb = rb.header("Schedule-Reply", "F");
                }
                if let Some(e) = etag {
                    rb = rb.header("If-Match", e.as_str());
                }
                let resp = self.dav.auth(rb)?.send().await?;
                let st = resp.status();
                if !(st.is_success() || st.as_u16() == 404) {
                    return Err(crate::net::http_error(st, &crate::net::error_text(resp).await, "CalDAV DELETE"));
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
