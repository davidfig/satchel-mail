//! How hosts serve the vendored golemdocs document viewer (0062): the files
//! under `ui/public/golemdocs/`, loaded in a sandboxed frame whose requests
//! all come from origin `null`. The desktop app serves them from its own URI
//! scheme and `satchel serve` under `/golemdocs/`; both take their headers
//! from here so the policy is written once.

/// Whether `path` (relative to the viewer folder) is one we serve: plain
/// names only, no `..`, no hidden files.
pub fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() < 256
        && path.split('/').all(|part| !part.is_empty() && !part.starts_with('.') && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
}

/// The type a viewer file is served with, by its extension. Anything we don't
/// expect in the build is plain text, which nothing runs.
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("webp") => "image/webp",
        Some("png") => "image/png",
        Some("ttf") => "font/ttf",
        Some("woff2") => "font/woff2",
        _ => "text/plain; charset=utf-8",
    }
}

/// Response headers for the viewer file at `path`, served from `origin`
/// (`golemdocs://localhost`, `http://127.0.0.1:7781`…). `frame_ancestor` is
/// the app's own origin where another page could try to frame the viewer
/// (the web); the desktop scheme is only reachable from the app's webview.
///
/// Every file but the page is readable from any origin: the sandboxed frame's
/// scripts, styles, worker and fonts are CORS requests from `null`. The page
/// itself names its origin rather than `'self'`, which WebKit matches to
/// nothing in an opaque frame, and allows `data:` workers, the only kind an
/// opaque origin can start. Nothing remote is allowed.
pub fn headers(path: &str, origin: &str, frame_ancestor: Option<&str>) -> Vec<(&'static str, String)> {
    let mut h = vec![("Content-Type", content_type(path).to_string()), ("X-Content-Type-Options", "nosniff".into())];
    if path.ends_with("viewer.html") {
        let ancestors = frame_ancestor.map(|a| format!("; frame-ancestors {a}")).unwrap_or_default();
        h.push((
            "Content-Security-Policy",
            format!(
                "default-src 'none'; script-src {origin}; worker-src {origin} data:; connect-src {origin}; style-src {origin}; \
                 img-src {origin} blob: data:; font-src {origin}; base-uri 'none'; form-action 'none'{ancestors}"
            ),
        ));
        h.push(("Referrer-Policy", "no-referrer".into()));
        h.push(("Cache-Control", "no-cache".into()));
    } else {
        h.push(("Access-Control-Allow-Origin", "*".into()));
        // The folder is named for the build, so its files never change.
        h.push(("Cache-Control", "public, max-age=31536000, immutable".into()));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(h: &'a [(&str, String)], name: &str) -> Option<&'a str> {
        h.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn only_plain_paths_are_served() {
        assert!(safe_path("4b3f750e1d72/viewer.html"));
        assert!(safe_path("4b3f750e1d72/fonts/calibri-regular.webp"));
        assert!(!safe_path("../secrets.json"));
        assert!(!safe_path("4b3f750e1d72/../../etc/passwd"));
        assert!(!safe_path("4b3f750e1d72//viewer.html"));
        assert!(!safe_path("/etc/passwd"));
        assert!(!safe_path(".git/config"));
        assert!(!safe_path("a/b%2e%2e/c"));
        assert!(!safe_path(""));
    }

    #[test]
    fn the_page_names_its_origin_and_nothing_remote() {
        let h = headers("x/viewer.html", "golemdocs://localhost", None);
        let csp = get(&h, "Content-Security-Policy").unwrap();
        assert!(csp.starts_with("default-src 'none'; script-src golemdocs://localhost;"));
        assert!(csp.contains("worker-src golemdocs://localhost data:;"));
        assert!(csp.contains("connect-src golemdocs://localhost;"));
        assert!(!csp.contains("'self'") && !csp.contains("http") && !csp.contains("frame-ancestors"));
        assert_eq!(get(&h, "Access-Control-Allow-Origin"), None);
        assert_eq!(get(&h, "Content-Type"), Some("text/html; charset=utf-8"));

        let web = headers("x/viewer.html", "http://127.0.0.1:7781", Some("http://127.0.0.1:7781"));
        assert!(get(&web, "Content-Security-Policy").unwrap().ends_with("; frame-ancestors http://127.0.0.1:7781"));
    }

    #[test]
    fn its_files_are_readable_from_the_sandbox() {
        let h = headers("x/assets/layoutCore-abc.js", "golemdocs://localhost", None);
        assert_eq!(get(&h, "Access-Control-Allow-Origin"), Some("*"));
        assert_eq!(get(&h, "Content-Type"), Some("text/javascript; charset=utf-8"));
        assert_eq!(get(&h, "Content-Security-Policy"), None);
        assert_eq!(content_type("x/fonts/manifest.json"), "application/json");
        assert_eq!(content_type("x/fonts/arial-regular.webp"), "image/webp");
        assert_eq!(content_type("x/LICENSE"), "text/plain; charset=utf-8");
    }
}
