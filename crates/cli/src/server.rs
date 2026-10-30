//! Localhost HTTP + WebSocket transport for the web UI and remote agents.
//!
//! Security: binds 127.0.0.1 only, rejects foreign Host headers (DNS
//! rebinding), and requires the session token as a bearer for user calls. Only
//! the link the server prints carries it; the page keeps it in its own storage.
//! There is no cookie: cookies ignore ports, so one would reach every other
//! server on 127.0.0.1. Browser calls must also come from our own origin.
//! Agents authenticate with their agent token.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use satchel_api::{Request as ApiRequest, Response as ApiResponse};
use satchel_core::{Caller, Engine};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct AppState {
    engine: Arc<Engine>,
    session: String,
    port: u16,
    /// Extra browser origin allowed to call the API (the Vite dev server).
    dev_origin: Option<String>,
}

/// How long a kept session token lasts before the server makes a new one.
const SESSION_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

/// The session token: `SATCHEL_SESSION_TOKEN`, else one kept (0600) in the
/// data directory so open tabs survive a server restart. A kept token is
/// replaced when it is older than 30 days or `new_session` is set, which signs
/// every browser out.
fn session_token(data_dir: &Path, new_session: bool) -> Result<String> {
    if let Ok(t) = std::env::var("SATCHEL_SESSION_TOKEN") {
        if t.len() < 32 {
            bail!("SATCHEL_SESSION_TOKEN must be at least 32 characters (e.g. `openssl rand -hex 16`)");
        }
        return Ok(t);
    }
    let path = data_dir.join("session-token");
    let fresh = |m: std::fs::Metadata| m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < SESSION_MAX_AGE);
    if !new_session
        && std::fs::metadata(&path).is_ok_and(fresh)
        && let Ok(t) = std::fs::read_to_string(&path)
        && t.trim().len() >= 32
    {
        return Ok(t.trim().to_string());
    }
    let t = satchel_core::random_token(24);
    satchel_core::engine::create_private_dir(data_dir)?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(&path)?, t.as_bytes())?;
    Ok(t)
}

pub async fn serve(engine: Arc<Engine>, port: u16, ui: Option<PathBuf>, new_session: bool) -> Result<()> {
    let data_dir = engine.data_dir.clone();
    let session = session_token(&data_dir, new_session)?;
    let dev_origin = std::env::var("SATCHEL_DEV_ORIGIN").ok().filter(|s| !s.is_empty());
    let state = AppState { engine, session: session.clone(), port, dev_origin };
    let mut app = Router::new()
        .route("/api/rpc", post(rpc))
        .route("/api/agent/rpc", post(agent_rpc))
        .route("/api/events", get(events))
        .route("/api/health", get(|| async { "ok" }));
    if let Some(dir) = ui {
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(&dir).not_found_service(ServeFile::new(index)));
    }
    let app = app
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    tracing::info!("satchel.email on http://127.0.0.1:{port}");
    // The link is the key: the page that opens it keeps the token.
    // The token rides in the fragment, which browsers never send or log.
    eprintln!("\nOpen satchel.email: http://127.0.0.1:{port}/#session={session}\n");
    let info = serde_json::json!({ "port": port, "pid": std::process::id() });
    let _ = std::fs::write(data_dir.join("server.json"), info.to_string());
    axum::serve(listener, app).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

/// Host check and headers.
async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let ok_hosts = [format!("127.0.0.1:{}", st.port), format!("localhost:{}", st.port)];
    if !ok_hosts.iter().any(|h| h == host) {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    let path = req.uri().path().to_string();
    let is_api = path.starts_with("/api/");
    // Nothing of ours is loaded in a frame, so a framed API request is a
    // clickjacking attempt.
    if is_api && framed(req.headers()) {
        return (StatusCode::FORBIDDEN, "no framing").into_response();
    }
    // The printed `#session=` link is read by the page itself, which keeps the
    // token in its own storage and drops it from the address bar.
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if !is_api {
        resp.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        if !path.starts_with("/assets/") {
            resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        }
        resp.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; img-src 'self' data: blob: https: http:; style-src 'self' 'unsafe-inline'; frame-src 'self' blob: data:; media-src blob:; connect-src 'self' ws://127.0.0.1:* ws://localhost:*; frame-ancestors 'none'"),
        );
    }
    resp
}

/// Whether the browser says this request loads into a frame.
fn framed(headers: &HeaderMap) -> bool {
    headers.get("sec-fetch-dest").and_then(|v| v.to_str().ok()).is_some_and(|d| d == "iframe" || d == "frame")
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn user_authorized(st: &AppState, headers: &HeaderMap) -> bool {
    session_authorized(&st.session, st.port, st.dev_origin.as_deref(), headers)
}

/// Whether a request may act as the user: it carries the session token as a
/// bearer (header, or the WebSocket subprotocol `bearer.<token>`, since
/// browsers can't set headers there) and, if a browser sent it, comes from our
/// own page. A foreign page can't send either without a CORS preflight we
/// never approve, or without reading our origin's storage.
fn session_authorized(session: &str, port: u16, dev_origin: Option<&str>, headers: &HeaderMap) -> bool {
    let header = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    if let Some(origin) = header("origin") {
        let ours = [format!("http://127.0.0.1:{port}"), format!("http://localhost:{port}")];
        if !ours.iter().any(|o| o == origin) && dev_origin != Some(origin) {
            return false;
        }
    }
    if header("sec-fetch-site").is_some_and(|s| s != "same-origin" && s != "none") {
        return false;
    }
    let token = session.as_bytes();
    let bearer = header("authorization").and_then(|v| v.strip_prefix("Bearer "));
    let subprotocol = headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .find_map(|p| p.trim().strip_prefix("bearer."));
    bearer.or(subprotocol).is_some_and(|b| constant_time_eq(b.as_bytes(), token))
}

async fn dispatch(st: &AppState, caller: &Caller, body: &[u8]) -> Response {
    let req: ApiRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("bad request: {e}")).into_response(),
    };
    let resp = match st.engine.handle(caller, req.command).await {
        Ok(v) => ApiResponse { id: req.id, ok: Some(v), error: None },
        Err(e) => ApiResponse { id: req.id, ok: None, error: Some(e) },
    };
    axum::Json(resp).into_response()
}

async fn rpc(State(st): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if !user_authorized(&st, &headers) {
        return (StatusCode::UNAUTHORIZED, "not signed in: open the link `satchel serve` printed").into_response();
    }
    // A JSON content type can't be sent cross-origin without a preflight.
    if !headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("application/json")) {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "expected application/json").into_response();
    }
    dispatch(&st, &Caller::User, &body).await
}

async fn agent_rpc(State(st): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    let Some(agent) = st.engine.authenticate_agent(token) else {
        return (StatusCode::UNAUTHORIZED, "invalid agent token").into_response();
    };
    dispatch(&st, &Caller::Agent(agent), &body).await
}

async fn events(State(st): State<AppState>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !user_authorized(&st, &headers) {
        return Response::builder().status(StatusCode::UNAUTHORIZED).body(Body::from("missing session")).unwrap();
    }
    // The browser sent `satchel` and `bearer.<token>`; answering with
    // `satchel` keeps the token out of the response.
    ws.protocols(["satchel"]).on_upgrade(move |socket| stream_events(socket, st.engine))
}

async fn stream_events(mut socket: WebSocket, engine: Arc<Engine>) {
    let mut rx = engine.subscribe();
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(e) => {
                    let Ok(s) = serde_json::to_string(&e) else { continue };
                    if socket.send(Message::Text(s.into())).await.is_err() { return; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let s = serde_json::json!({ "event": "data_changed" }).to_string();
                    if socket.send(Message::Text(s.into())).await.is_err() { return; }
                }
                Err(_) => return,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn ok(pairs: &[(&'static str, &str)]) -> bool {
        session_authorized(TOKEN, 7781, Some("http://127.0.0.1:7780"), &headers(pairs))
    }

    #[test]
    fn framed_requests_are_spotted() {
        assert!(framed(&headers(&[("sec-fetch-dest", "iframe")])));
        assert!(framed(&headers(&[("sec-fetch-dest", "frame")])));
        assert!(!framed(&headers(&[("sec-fetch-dest", "empty")])));
        assert!(!framed(&headers(&[])));
    }

    #[test]
    fn cookies_are_not_a_session() {
        // Cookies reach every server on 127.0.0.1, so they never count.
        let cookie = format!("satchel_session={TOKEN}");
        assert!(!ok(&[("cookie", &cookie)]));
        assert!(!ok(&[("cookie", &cookie), ("origin", "http://127.0.0.1:7781")]));
    }

    #[test]
    fn websocket_subprotocol_carries_the_token() {
        let protos = format!("satchel, bearer.{TOKEN}");
        assert!(ok(&[("sec-websocket-protocol", &protos), ("origin", "http://127.0.0.1:7781")]));
        assert!(!ok(&[("sec-websocket-protocol", &protos), ("origin", "http://127.0.0.1:9999")]));
        assert!(!ok(&[("sec-websocket-protocol", "satchel, bearer.nope"), ("origin", "http://127.0.0.1:7781")]));
    }

    #[test]
    fn bearer_works_without_a_browser_but_not_from_foreign_pages() {
        let bearer = format!("Bearer {TOKEN}");
        assert!(ok(&[("authorization", &bearer)]));
        assert!(ok(&[("authorization", &bearer), ("origin", "http://127.0.0.1:7780")]));
        assert!(!ok(&[("authorization", &bearer), ("origin", "http://evil.test")]));
        assert!(!ok(&[("authorization", &bearer), ("sec-fetch-site", "cross-site")]));
        assert!(!ok(&[("authorization", "Bearer nope")]));
        assert!(!ok(&[]));
    }
}
