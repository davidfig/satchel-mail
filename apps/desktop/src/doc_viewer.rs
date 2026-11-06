//! The `golemdocs:` URI scheme: serves the vendored document viewer (0062)
//! with the headers its sandboxed frame needs, which the app's own scheme
//! can't set per file. It is its own origin (`golemdocs://localhost`, or
//! `http://golemdocs.localhost` on Windows), apart from the app's pages and IPC.

use std::borrow::Cow;

use tauri::http::{Request, Response, StatusCode};
use tauri::{Runtime, UriSchemeContext, UriSchemeResponder};

pub const SCHEME: &str = "golemdocs";

/// Answers one request off the main thread: the fonts run to megabytes.
pub fn handle<R: Runtime>(ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    std::thread::spawn(move || responder.respond(respond(&app, &request)));
}

fn respond<R: Runtime>(app: &tauri::AppHandle<R>, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    let path = uri.path().trim_start_matches('/');
    let not_found = || Response::builder().status(StatusCode::NOT_FOUND).body(Cow::Borrowed(&b""[..])).unwrap();
    if !satchel_core::doc_viewer::safe_path(path) {
        return not_found();
    }
    let Some(bytes) = read(app, path) else { return not_found() };
    let origin = format!("{}://{}", uri.scheme_str().unwrap_or(SCHEME), uri.host().unwrap_or("localhost"));
    let mut response = Response::builder();
    for (name, value) in satchel_core::doc_viewer::headers(path, &origin, None) {
        response = response.header(name, value);
    }
    response.body(Cow::Owned(bytes)).unwrap_or_else(|_| not_found())
}

/// The file from the app's embedded assets (`ui/dist/golemdocs/…`). A dev
/// build reads them from `ui/public/`, since `tauri dev` serves the UI from
/// Vite and `ui/dist` may be stale or missing.
fn read<R: Runtime>(app: &tauri::AppHandle<R>, path: &str) -> Option<Vec<u8>> {
    if cfg!(debug_assertions) {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/public/golemdocs").join(path);
        if let Ok(bytes) = std::fs::read(source) {
            return Some(bytes);
        }
    }
    let asset = app.asset_resolver().get(format!("golemdocs/{path}"))?;
    // A missing asset comes back as the app's index.html; never serve that.
    (path.ends_with(".html") || !asset.mime_type.starts_with("text/html")).then_some(asset.bytes)
}

/// Whether `url` is one of the viewer's (its frame navigating to its page).
pub fn is_viewer(url: &tauri::Url) -> bool {
    url.scheme() == SCHEME || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("golemdocs.localhost"))
}
