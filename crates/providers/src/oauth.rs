//! OAuth 2.0 for installed apps: authorization code + PKCE with a loopback
//! redirect (RFC 8252), plus refresh-token handling.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use satchel_api::*;
use satchel_core::provider::{CredentialHandle, OAuthDone, priority};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct OAuthApp {
    pub provider: ProviderKind,
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    /// Desktop clients' secrets are not confidential (RFC 8252 §8.5) but some
    /// providers (Google) still require them.
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    pub extra_auth_params: Vec<(String, String)>,
}

impl OAuthApp {
    pub fn google() -> Option<Self> {
        let client_id = std::env::var("SATCHEL_GOOGLE_CLIENT_ID").ok().or_else(|| option_env!("SATCHEL_GOOGLE_CLIENT_ID").map(String::from))?;
        let client_secret = std::env::var("SATCHEL_GOOGLE_CLIENT_SECRET").ok().or_else(|| option_env!("SATCHEL_GOOGLE_CLIENT_SECRET").map(String::from));
        Some(Self {
            provider: ProviderKind::Gmail,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            client_id,
            client_secret,
            scopes: vec![
                "openid".into(),
                "email".into(),
                "profile".into(),
                "https://mail.google.com/".into(),
                "https://www.googleapis.com/auth/calendar".into(),
                "https://www.googleapis.com/auth/contacts".into(),
            ],
            extra_auth_params: vec![("access_type".into(), "offline".into()), ("prompt".into(), "consent".into())],
        })
    }

    pub fn microsoft() -> Option<Self> {
        let client_id = std::env::var("SATCHEL_MS_CLIENT_ID").ok().or_else(|| option_env!("SATCHEL_MS_CLIENT_ID").map(String::from))?;
        let tenant = std::env::var("SATCHEL_MS_TENANT").unwrap_or_else(|_| "common".into());
        Some(Self {
            provider: ProviderKind::Graph,
            auth_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"),
            token_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
            client_id,
            client_secret: None,
            scopes: vec![
                "offline_access".into(),
                "openid".into(),
                "email".into(),
                "profile".into(),
                "https://graph.microsoft.com/Mail.ReadWrite".into(),
                "https://graph.microsoft.com/Mail.Send".into(),
                "https://graph.microsoft.com/Calendars.ReadWrite".into(),
                "https://graph.microsoft.com/Contacts.ReadWrite".into(),
                "https://graph.microsoft.com/User.Read".into(),
            ],
            extra_auth_params: vec![("prompt".into(), "select_account".into())],
        })
    }

    pub fn for_kind(kind: ProviderKind) -> Option<Self> {
        match kind {
            ProviderKind::Gmail => Self::google(),
            ProviderKind::Graph => Self::microsoft(),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    #[allow(dead_code)]
    id_token: Option<String>,
}

fn sha256_b64(s: &str) -> String {
    // Minimal SHA-256 for the PKCE challenge (avoids another dependency).
    let digest = sha256(s.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

fn query_param(q: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
}

/// Begin a sign-in. Returns the URL to open; `done` runs after the redirect.
pub fn start(app: OAuthApp, done: OAuthDone) -> Result<String> {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").context("cannot open a local port for sign-in")?;
    std_listener.set_nonblocking(true)?;
    let port = std_listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let verifier = satchel_core::random_token(48);
    let state = satchel_core::random_token(16);
    let mut url = url::Url::parse(&app.auth_url)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &app.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", &app.scopes.join(" "))
            .append_pair("state", &state)
            .append_pair("code_challenge", &sha256_b64(&verifier))
            .append_pair("code_challenge_method", "S256");
        for (k, v) in &app.extra_auth_params {
            q.append_pair(k, v);
        }
    }
    let auth_url = url.to_string();
    tokio::spawn(async move {
        let res = tokio::time::timeout(Duration::from_secs(600), finish(app, std_listener, redirect, verifier, state)).await;
        let res = match res {
            Ok(r) => r,
            Err(_) => Err(anyhow!("sign-in timed out")),
        };
        done(res);
    });
    Ok(auth_url)
}

async fn finish(app: OAuthApp, listener: std::net::TcpListener, redirect: String, verifier: String, state: String) -> Result<(AccountConfig, Credential)> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    // Each connection reads its request line in its own task, so one that
    // connects and says nothing can't hold up the browser's real callback.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, tokio::net::TcpStream)>(8);
    let (code, mut sock) = loop {
        let (path, mut sock) = tokio::select! {
            accepted = listener.accept() => {
                let (mut sock, _) = accepted?;
                let tx = tx.clone();
                tokio::spawn(async move {
                    if let Some(path) = request_path(&mut sock, REQUEST_TIMEOUT).await {
                        let _ = tx.send((path, sock)).await;
                    }
                });
                continue;
            }
            Some(req) = rx.recv() => req,
        };
        let Some(q) = path.strip_prefix("/callback?") else {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        };
        // Anything on this computer can reach the port: a callback that isn't
        // for this sign-in is turned away, and we keep waiting for the real one.
        if query_param(q, "state").as_deref() != Some(state.as_str()) {
            respond(&mut sock, "Sign-in failed", "The response didn't match this sign-in attempt. Please try again from satchel.email.").await;
            continue;
        }
        if let Some(err) = query_param(q, "error") {
            let desc = query_param(q, "error_description").unwrap_or_default();
            respond(&mut sock, "Sign-in cancelled", "You can close this tab.").await;
            bail!("{err}: {desc}");
        }
        let code = query_param(q, "code").context("no authorization code")?;
        break (code, sock);
    };
    let http = crate::net::http_client(false)?;
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", app.client_id.clone()),
        ("code_verifier", verifier),
    ];
    if let Some(s) = &app.client_secret {
        form.push(("client_secret", s.clone()));
    }
    let resp = http.post(&app.token_url).form(&form).send().await?;
    let status = resp.status();
    let body = crate::net::text_capped(resp, crate::net::API_LIMIT).await?;
    if !status.is_success() {
        respond(&mut sock, "Sign-in failed", "The provider rejected the sign-in. Please try again.").await;
        return Err(crate::net::http_error(status, &body, "token exchange"));
    }
    let tok: TokenResponse = serde_json::from_str(&body)?;
    let (email, name) = identity(&http, app.provider, &tok.access_token).await?;
    respond(&mut sock, "You're signed in", "Return to satchel.email — your account is syncing. You can close this tab.").await;
    let cred = Credential::OAuth { access_token: tok.access_token, refresh_token: tok.refresh_token, expires_at: tok.expires_in.map(|e| now() + e) };
    let config = match app.provider {
        ProviderKind::Gmail => AccountConfig::Gmail { email, display_name: name },
        ProviderKind::Graph => AccountConfig::Graph { email, display_name: name },
        _ => bail!("unsupported OAuth provider"),
    };
    Ok((config, cred))
}

/// How long a connection to the sign-in listener has to send its request line.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The path of an HTTP request, once its whole request line has arrived
/// (within `limit`, and 8 KiB). `None` for a slow, silent or malformed one.
async fn request_path<S: tokio::io::AsyncRead + Unpin>(sock: &mut S, limit: Duration) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let line = tokio::time::timeout(limit, async {
        loop {
            if let Some(end) = buf.iter().position(|&b| b == b'\n') {
                return Some(String::from_utf8_lossy(&buf[..end]).into_owned());
            }
            if buf.len() >= 8192 {
                return None;
            }
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    })
    .await
    .ok()??;
    line.split_whitespace().nth(1).map(String::from)
}

async fn respond(sock: &mut tokio::net::TcpStream, title: &str, msg: &str) {
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title><body style=\"font:16px -apple-system,sans-serif;display:grid;place-items:center;height:90vh;color:#333\"><div style=text-align:center><h1 style=font-weight:600>{title}</h1><p>{msg}</p></div>"
    );
    let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len());
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.shutdown().await;
}

async fn identity(http: &reqwest::Client, kind: ProviderKind, token: &str) -> Result<(String, String)> {
    match kind {
        ProviderKind::Gmail => {
            let r = http.get("https://openidconnect.googleapis.com/v1/userinfo").bearer_auth(token).send().await?.error_for_status()?;
            let v: serde_json::Value = serde_json::from_str(&crate::net::text_capped(r, crate::net::API_LIMIT).await?)?;
            Ok((v["email"].as_str().context("no email in profile")?.to_string(), v["name"].as_str().unwrap_or_default().to_string()))
        }
        ProviderKind::Graph => {
            let r = http.get("https://graph.microsoft.com/v1.0/me?$select=displayName,mail,userPrincipalName").bearer_auth(token).send().await?.error_for_status()?;
            let v: serde_json::Value = serde_json::from_str(&crate::net::text_capped(r, crate::net::API_LIMIT).await?)?;
            let email = v["mail"].as_str().or(v["userPrincipalName"].as_str()).context("no email in profile")?;
            Ok((email.to_string(), v["displayName"].as_str().unwrap_or_default().to_string()))
        }
        _ => bail!("unsupported"),
    }
}

/// Supplies fresh access tokens, refreshing and persisting as needed.
pub struct TokenSource {
    app: Option<OAuthApp>,
    creds: CredentialHandle,
    http: reqwest::Client,
    lock: Arc<Mutex<()>>,
}

impl TokenSource {
    pub fn new(app: Option<OAuthApp>, creds: CredentialHandle) -> Result<Self> {
        Ok(Self { app, creds, http: crate::net::http_client(false)?, lock: Arc::new(Mutex::new(())) })
    }

    pub async fn token(&self) -> Result<String> {
        let _g = self.lock.lock().await;
        match self.creds.get()? {
            Credential::OAuth { access_token, refresh_token, expires_at } => {
                if expires_at.is_none_or(|e| e > now() + 90) {
                    return Ok(access_token);
                }
                let rt = refresh_token.clone().ok_or_else(|| anyhow::Error::new(ApiError::new(ErrorCode::Auth, "session expired; sign in again")))?;
                self.refresh(&rt).await
            }
            Credential::Password { password } => Ok(password),
        }
    }

    /// Force a refresh (after a 401).
    pub async fn refresh_now(&self) -> Result<String> {
        let _g = self.lock.lock().await;
        match self.creds.get()? {
            Credential::OAuth { refresh_token: Some(rt), .. } => self.refresh(&rt).await,
            _ => Err(anyhow::Error::new(ApiError::new(ErrorCode::Auth, "session expired; sign in again"))),
        }
    }

    async fn refresh(&self, refresh_token: &str) -> Result<String> {
        let app = self.app.as_ref().ok_or_else(|| anyhow!("OAuth client is not configured"))?;
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
            ("client_id", app.client_id.clone()),
        ];
        if let Some(s) = &app.client_secret {
            form.push(("client_secret", s.clone()));
        }
        if app.provider == ProviderKind::Graph {
            form.push(("scope", app.scopes.join(" ")));
        }
        let resp = self.http.post(&app.token_url).form(&form).send().await?;
        let status = resp.status();
        let body = crate::net::text_capped(resp, crate::net::API_LIMIT).await?;
        if !status.is_success() {
            let mut e = crate::net::http_error(status, &body, "token refresh");
            if status.as_u16() == 400 && body.contains("invalid_grant") {
                e = anyhow::Error::new(ApiError::new(ErrorCode::Auth, "access was revoked; sign in again"));
            }
            return Err(e);
        }
        let tok: TokenResponse = serde_json::from_str(&body)?;
        let cred = Credential::OAuth {
            access_token: tok.access_token.clone(),
            refresh_token: tok.refresh_token.or_else(|| Some(refresh_token.to_string())),
            expires_at: tok.expires_in.map(|e| now() + e),
        };
        self.creds.set(&cred)?;
        Ok(tok.access_token)
    }
}

/// The longest one rate-limit backoff may hold up a request the user is waiting on.
const USER_MAX_BACKOFF: Duration = Duration::from_secs(4);

/// An authenticated JSON HTTP client that retries once on 401 after a refresh
/// and backs off on 429/503 and Google's quota 403s. Background requests wait
/// while the user is waiting on the account; the user's own requests back off
/// briefly instead of sitting out a long Retry-After (0037).
pub struct ApiClient {
    pub http: reqwest::Client,
    pub tokens: TokenSource,
}

impl ApiClient {
    pub async fn send(&self, build: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder, what: &str) -> Result<reqwest::Response> {
        let mut token = self.tokens.token().await?;
        let mut refreshed = false;
        let mut backoff = Duration::from_millis(500);
        let max_wait = if priority::is_user() { USER_MAX_BACKOFF } else { Duration::from_secs(60) };
        for _ in 0..6 {
            priority::yield_to_user().await;
            let req = build(&self.http, &token).build().with_context(|| format!("{what}: bad request"))?;
            let sends_mail = sends_mail(&req);
            let resp = self.http.execute(req).await.with_context(|| format!("{what}: request failed"))?;
            let st = resp.status();
            if st.as_u16() == 401 && !refreshed {
                token = self.tokens.refresh_now().await?;
                refreshed = true;
                continue;
            }
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            if st.is_success() {
                return Ok(resp);
            }
            // A 503 may come after the server took the message: sending it
            // again could deliver it twice, so the op queue decides (0030).
            // A 429 or quota refusal means it wasn't processed.
            if st.as_u16() != 429 && (st.as_u16() != 503 || sends_mail) {
                let body = crate::net::error_text(resp).await;
                // Google sends per-minute quota errors as 403.
                if !(st.as_u16() == 403 && crate::net::is_quota_body(&body)) {
                    return Err(crate::net::http_error(st, &body, what));
                }
            }
            tokio::time::sleep(retry_after.unwrap_or(backoff).min(max_wait)).await;
            backoff *= 2;
        }
        Err(anyhow::Error::new(ApiError::new(ErrorCode::RateLimited, format!("{what}: rate limited"))))
    }

    pub async fn json(&self, build: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder, what: &str) -> Result<serde_json::Value> {
        let r = self.send(build, what).await?;
        // Gmail sends whole raw messages inside JSON.
        let text = crate::net::text_capped(r, crate::net::DOWNLOAD_LIMIT).await?;
        if text.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }
        Ok(serde_json::from_str(&text)?)
    }
}

/// Whether a request sends mail (Gmail `messages/send`, `drafts/send`; Graph
/// `sendMail`, `messages/{id}/send`, `reply`, `forward`): repeating one
/// after an unclear failure could send the message twice.
fn sends_mail(req: &reqwest::Request) -> bool {
    req.method() == reqwest::Method::POST
        && req.url().path_segments().and_then(|mut s| s.next_back()).is_some_and(|last| ["send", "sendMail", "reply", "replyAll", "forward"].contains(&last))
}

#[cfg(test)]
mod tests {
    #[test]
    fn credentials_debug_hides_secrets() {
        use satchel_api::Credential;
        let p = format!("{:?}", Credential::Password { password: "hunter2".into() });
        let o = format!("{:?}", Credential::OAuth { access_token: "ya29.tok".into(), refresh_token: Some("1//ref".into()), expires_at: Some(5) });
        assert!(!p.contains("hunter2") && !o.contains("ya29") && !o.contains("1//ref"), "{p} {o}");
        assert!(o.contains("expires_at: Some(5)"));
    }

    #[tokio::test]
    async fn listener_reads_the_whole_request_line() {
        use std::time::Duration;
        use tokio::io::AsyncWriteExt;
        let (mut client, mut server) = tokio::io::duplex(64);
        let reader = tokio::spawn(async move { super::request_path(&mut server, Duration::from_secs(5)).await });
        client.write_all(b"GET /callback?co").await.unwrap();
        tokio::task::yield_now().await;
        client.write_all(b"de=1&state=x HTTP/1.1\r\nHost: a\r\n\r\n").await.unwrap();
        assert_eq!(reader.await.unwrap().as_deref(), Some("/callback?code=1&state=x"));
        // A connection that says nothing gives up instead of waiting forever.
        let (_quiet, mut server) = tokio::io::duplex(64);
        assert_eq!(super::request_path(&mut server, Duration::from_millis(50)).await, None);
    }

    #[test]
    fn send_endpoints_are_not_replayed() {
        let http = reqwest::Client::new();
        let is = |m: reqwest::Method, u: &str| super::sends_mail(&http.request(m, u).build().unwrap());
        assert!(is(reqwest::Method::POST, "https://gmail.googleapis.com/gmail/v1/users/me/messages/send"));
        assert!(is(reqwest::Method::POST, "https://gmail.googleapis.com/upload/gmail/v1/users/me/messages/send?uploadType=media"));
        assert!(is(reqwest::Method::POST, "https://graph.microsoft.com/v1.0/me/sendMail"));
        assert!(is(reqwest::Method::POST, "https://graph.microsoft.com/v1.0/me/messages/AAk%3D/send"));
        assert!(!is(reqwest::Method::POST, "https://gmail.googleapis.com/gmail/v1/users/me/messages/abc/modify"));
        assert!(!is(reqwest::Method::GET, "https://graph.microsoft.com/v1.0/me/messages/send"));
    }

    #[test]
    fn sha256_known() {
        let h = super::sha256(b"abc");
        assert_eq!(h[..4], [0xba, 0x78, 0x16, 0xbf]);
        // RFC 7636 appendix B example
        assert_eq!(super::sha256_b64("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
