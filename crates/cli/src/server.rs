//! Localhost HTTP + WebSocket transport for the web UI and remote agents.
//!
//! Security: binds 127.0.0.1 only, rejects foreign Host headers (DNS
//! rebinding), and requires a session cookie (SameSite=Strict) or bearer token
//! for user calls. The cookie is only handed out to whoever opens the link the
//! server prints (it carries the session token), never to any page or local
//! program that asks. Browser calls must come from our own origin: SameSite
//! doesn't separate ports, so a page on another localhost port is "same-site".
//! Agents authenticate with their agent token.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
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

/// The session token: `SATCHEL_SESSION_TOKEN`, else one kept (0600) in the
/// data directory so open tabs survive a server restart.
fn session_token() -> Result<String> {
    if let Ok(t) = std::env::var("SATCHEL_SESSION_TOKEN") {
        return Ok(t);
    }
    let path = satchel_core::default_data_dir().join("session-token");
    if let Ok(t) = std::fs::read_to_string(&path)
        && t.trim().len() >= 32
    {
        return Ok(t.trim().to_string());
    }
    let t = satchel_core::random_token(24);
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(&path)?, t.as_bytes())?;
    Ok(t)
}

pub async fn serve(engine: Arc<Engine>, port: u16, ui: Option<PathBuf>) -> Result<()> {
    let session = session_token()?;
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
    tracing::info!("Satchel Mail on http://127.0.0.1:{port}");
    // The link is the key: it sets the session cookie in the browser that opens it.
    eprintln!("\nOpen Satchel Mail: http://127.0.0.1:{port}/?session={session}\n");
    let info = serde_json::json!({ "port": port, "pid": std::process::id() });
    let _ = std::fs::write(state_path(&listener), info.to_string());
    axum::serve(listener, app).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

fn state_path(_l: &tokio::net::TcpListener) -> PathBuf {
    satchel_core::default_data_dir().join("server.json")
}

/// Host check, cookie issuance from the printed link, and headers.
async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let ok_hosts = [format!("127.0.0.1:{}", st.port), format!("localhost:{}", st.port)];
    if !ok_hosts.iter().any(|h| h == host) {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    let path = req.uri().path().to_string();
    let is_api = path.starts_with("/api/");
    if !is_api && let Some(q) = req.uri().query() {
        let given = q.split('&').find_map(|kv| kv.strip_prefix("session="));
        if let Some(given) = given {
            if !constant_time_eq(given.as_bytes(), st.session.as_bytes()) {
                return (StatusCode::FORBIDDEN, "this link is out of date; use the one `satchel serve` printed").into_response();
            }
            // Set the cookie and drop the token from the address bar.
            let cookie = format!("satchel_session={}; HttpOnly; SameSite=Strict; Path=/", st.session);
            let mut resp = Response::builder().status(StatusCode::SEE_OTHER).header(header::LOCATION, path).body(Body::empty()).unwrap();
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().append(header::SET_COOKIE, v);
            }
            resp.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
            return resp;
        }
    }
    let mut resp = next.run(req).await;
    if !is_api {
        if !path.starts_with("/assets/") {
            resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        }
        resp.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; img-src 'self' data: https: http:; style-src 'self' 'unsafe-inline'; frame-src 'self' blob: data:; connect-src 'self' ws://127.0.0.1:* ws://localhost:*"),
        );
    }
    resp
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether a request may act as the user: it carries the session (cookie or
/// bearer) and, if a browser sent it, comes from our own page. Browsers always
/// send `Origin` on POSTs and WebSockets, and `Sec-Fetch-Site` when they know.
fn user_authorized(st: &AppState, headers: &HeaderMap) -> bool {
    let header = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    if let Some(origin) = header("origin") {
        let ours = [format!("http://127.0.0.1:{}", st.port), format!("http://localhost:{}", st.port)];
        if !ours.iter().any(|o| o == origin) && st.dev_origin.as_deref() != Some(origin) {
            return false;
        }
    }
    if header("sec-fetch-site").is_some_and(|s| s != "same-origin" && s != "none") {
        return false;
    }
    let token = st.session.as_bytes();
    if header("authorization").and_then(|v| v.strip_prefix("Bearer ")).is_some_and(|b| constant_time_eq(b.as_bytes(), token)) {
        return true;
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().strip_prefix("satchel_session="))
        .any(|c| constant_time_eq(c.as_bytes(), token))
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
    ws.on_upgrade(move |socket| stream_events(socket, st.engine))
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
