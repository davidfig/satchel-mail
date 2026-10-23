//! Localhost HTTP + WebSocket transport for the web UI and remote agents.
//!
//! Security: binds 127.0.0.1 only, rejects foreign Host headers (DNS
//! rebinding), and requires a per-run session cookie (SameSite=Strict) or
//! bearer token for user calls. Agents authenticate with their agent token.

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
use base64::Engine as _;
use satchel_api::{Request as ApiRequest, Response as ApiResponse};
use satchel_core::{Caller, Engine};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct AppState {
    engine: Arc<Engine>,
    session: String,
    port: u16,
}

fn random_token() -> String {
    let mut b = [0u8; 24];
    getrandom(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn getrandom(buf: &mut [u8]) {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom")
        && f.read_exact(buf).is_ok() {
            return;
        }
    // Fallback: time + address entropy (non-Unix dev only).
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    for (i, b) in buf.iter_mut().enumerate() {
        *b = ((seed >> ((i % 16) * 8)) as u8) ^ (i as u8).wrapping_mul(31);
    }
}

pub async fn serve(engine: Arc<Engine>, port: u16, ui: Option<PathBuf>) -> Result<()> {
    let session = std::env::var("SATCHEL_SESSION_TOKEN").unwrap_or_else(|_| random_token());
    let state = AppState { engine, session: session.clone(), port };
    let mut app = Router::new()
        .route("/api/rpc", post(rpc))
        .route("/api/agent/rpc", post(agent_rpc))
        .route("/api/events", get(events))
        .route("/api/health", get(|| async { "ok" }))
        // Issues a fresh session cookie (e.g. after the server restarted).
        .route("/api/session", get(|| async { "ok" }));
    if let Some(dir) = ui {
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(&dir).not_found_service(ServeFile::new(index)));
    }
    let app = app
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    tracing::info!("Satchel Mail on http://127.0.0.1:{port}  (session token in SATCHEL_SESSION_TOKEN for scripts)");
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

/// Host check + session cookie issuance.
async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let ok_hosts = [format!("127.0.0.1:{}", st.port), format!("localhost:{}", st.port)];
    if !ok_hosts.iter().any(|h| h == host) {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    let path = req.uri().path().to_string();
    let is_api = path.starts_with("/api/") && path != "/api/session";
    let mut resp = next.run(req).await;
    if !is_api {
        let cookie = format!("satchel_session={}; HttpOnly; SameSite=Strict; Path=/", st.session);
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
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

fn user_authorized(st: &AppState, headers: &HeaderMap) -> bool {
    let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    if bearer == Some(st.session.as_str()) {
        return true;
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .any(|c| c.trim() == format!("satchel_session={}", st.session))
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
        return (StatusCode::UNAUTHORIZED, "missing session").into_response();
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
