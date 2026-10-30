//! TCP/TLS plumbing shared by IMAP and HTTP-based providers.

use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

fn roots() -> RootCertStore {
    static ROOTS: OnceLock<RootCertStore> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut r = RootCertStore::empty();
            let res = rustls_native_certs::load_native_certs();
            for c in res.certs {
                let _ = r.add(c);
            }
            r
        })
        .clone()
}

/// Accepts any certificate. Only used when the account explicitly opts in
/// (self-signed local servers).
#[derive(Debug)]
struct AcceptAll(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAll {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

pub fn tls_config(allow_invalid: bool) -> Arc<ClientConfig> {
    let p = provider();
    let builder = ClientConfig::builder_with_provider(p.clone()).with_safe_default_protocol_versions().expect("tls versions");
    let cfg = if allow_invalid {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(AcceptAll(p))).with_no_client_auth()
    } else {
        builder.with_root_certificates(roots()).with_no_client_auth()
    };
    Arc::new(cfg)
}

pub async fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    let s = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| anyhow!("connection to {host}:{port} timed out"))?
        .with_context(|| format!("connection to {host}:{port} failed"))?;
    s.set_nodelay(true)?;
    Ok(s)
}

pub async fn tls_wrap(s: TcpStream, host: &str, allow_invalid: bool) -> Result<TlsStream<TcpStream>> {
    let name = ServerName::try_from(host.to_string()).map_err(|_| anyhow!("invalid host name {host}"))?;
    let c = TlsConnector::from(tls_config(allow_invalid));
    tokio::time::timeout(CONNECT_TIMEOUT, c.connect(name, s))
        .await
        .map_err(|_| anyhow!("TLS handshake with {host} timed out"))?
        .with_context(|| format!("TLS handshake with {host} failed"))
}

/// A connection that may or may not be wrapped in TLS.
pub enum MaybeTls {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl std::fmt::Debug for MaybeTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaybeTls::Plain(_) => write!(f, "Plain"),
            MaybeTls::Tls(_) => write!(f, "Tls"),
        }
    }
}

impl AsyncRead for MaybeTls {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTls {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_flush(cx),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// A stream whose reads and writes fail when the other side goes quiet for
/// `stall` while we wait on it. Without it a half-open connection (after the
/// laptop sleeps or the network changes) hangs forever. `None` never times
/// out, for connections that wait quietly on purpose (IMAP IDLE).
///
/// A server can also dodge the silence check by trickling a byte now and
/// then, so each response (everything read between two of our writes) must
/// also average [`TRICKLE_MIN_BYTES`] per [`TRICKLE_WINDOW`]. Real progress
/// updates on long commands send far more.
pub struct Timed<S> {
    inner: S,
    stall: Option<Duration>,
    read_deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    write_deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    /// Start of the current throughput window and bytes read in it.
    window: Option<(tokio::time::Instant, u64)>,
}

pub const TRICKLE_WINDOW: Duration = Duration::from_secs(600);
pub const TRICKLE_MIN_BYTES: u64 = 1024;

impl<S> Timed<S> {
    pub fn new(inner: S, stall: Option<Duration>) -> Self {
        Self { inner, stall, read_deadline: None, write_deadline: None, window: None }
    }
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: std::fmt::Debug> std::fmt::Debug for Timed<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

/// Pending I/O: arm the deadline, and fail once it passes.
fn stalled<T>(deadline: &mut Option<Pin<Box<tokio::time::Sleep>>>, stall: Option<Duration>, cx: &mut Context<'_>) -> Poll<std::io::Result<T>> {
    let Some(stall) = stall else { return Poll::Pending };
    let sleep = deadline.get_or_insert_with(|| Box::pin(tokio::time::sleep(stall)));
    match std::future::Future::poll(sleep.as_mut(), cx) {
        Poll::Ready(()) => {
            *deadline = None;
            Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("the server stopped responding for {}s", stall.as_secs()))))
        }
        Poll::Pending => Poll::Pending,
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Timed<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Pending => stalled(&mut this.read_deadline, this.stall, cx),
            Poll::Ready(Ok(())) => {
                this.read_deadline = None;
                if this.stall.is_some() {
                    trickled(&mut this.window, (buf.filled().len() - before) as u64, tokio::time::Instant::now())?;
                }
                Poll::Ready(Ok(()))
            }
            ready => {
                this.read_deadline = None;
                ready
            }
        }
    }
}

/// Count `n` bytes read at `now` into the throughput window; fail when a
/// whole window passed with too few.
fn trickled(window: &mut Option<(tokio::time::Instant, u64)>, n: u64, now: tokio::time::Instant) -> std::io::Result<()> {
    let (start, bytes) = window.get_or_insert((now, 0));
    *bytes += n;
    if now.duration_since(*start) >= TRICKLE_WINDOW {
        if *bytes < TRICKLE_MIN_BYTES {
            *window = None;
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the server is sending too slowly"));
        }
        *window = Some((now, 0));
    }
    Ok(())
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Timed<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        // A new command: its response gets a fresh window.
        this.window = None;
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Pending => stalled(&mut this.write_deadline, this.stall, cx),
            ready => {
                this.write_deadline = None;
                ready
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Pending => stalled(&mut this.write_deadline, this.stall, cx),
            ready => {
                this.write_deadline = None;
                ready
            }
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// HTTP client honoring the account's certificate policy.
pub fn http_client(allow_invalid: bool) -> Result<reqwest::Client> {
    Ok(http_builder(allow_invalid).build()?)
}

/// [`http_client`]'s settings, for clients that need more (redirect policy, resolver).
pub fn http_builder(allow_invalid: bool) -> reqwest::ClientBuilder {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("SatchelMail/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(Duration::from_secs(120));
    if allow_invalid {
        b = b.tls_danger_accept_invalid_certs(true);
    }
    b
}

/// Cap for small control responses: tokens, sessions, error bodies.
pub const API_LIMIT: usize = 16 << 20;
/// Cap for API responses that carry data (DAV multistatus, JMAP method calls).
pub const DATA_LIMIT: usize = 64 << 20;
/// Cap for whole messages and attachments (Graph allows 150 MB messages, and
/// Gmail sends raw messages base64-encoded inside JSON).
pub const DOWNLOAD_LIMIT: usize = 160 << 20;

/// Reads a response body, failing once it passes `limit` bytes, so a hostile
/// or broken server can't fill memory.
pub async fn body_capped(mut resp: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let too_big = || anyhow!("the server's response is larger than {} MiB", limit >> 20);
    if resp.content_length().is_some_and(|n| n > limit as u64) {
        return Err(too_big());
    }
    let mut out = Vec::with_capacity(resp.content_length().unwrap_or(0) as usize);
    while let Some(chunk) = resp.chunk().await? {
        if out.len() + chunk.len() > limit {
            return Err(too_big());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// [`body_capped`] as text (JSON and XML APIs send UTF-8).
pub async fn text_capped(resp: reqwest::Response, limit: usize) -> Result<String> {
    let bytes = body_capped(resp, limit).await?;
    Ok(String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

/// The body of an error response, for the message ([`http_error`]).
pub async fn error_text(resp: reqwest::Response) -> String {
    text_capped(resp, API_LIMIT).await.unwrap_or_default()
}

/// Whether `url` may receive the credentials of an account whose server is at
/// `base`: the same origin (scheme, host, port), or another https host of
/// the same site when `base` is https too (iCloud moves each user's calendars
/// from caldav.icloud.com to pNN-caldav.icloud.com). Never a downgrade to
/// plain http.
pub fn credentials_allowed(base: &url::Url, url: &url::Url) -> bool {
    if base.origin() == url.origin() {
        return true;
    }
    let (Some(a), Some(b)) = (base.host_str(), url.host_str()) else { return false };
    base.scheme() == "https" && url.scheme() == "https" && url.port_or_known_default() == Some(443) && site(a).is_some_and(|s| Some(s) == site(b))
}

/// Suffixes under which unrelated customers each get their own names
/// (`alice.github.io`, `bob.duckdns.org`). Hosts under them share a site only
/// with their own origin. A short built-in stand-in for the Public Suffix List.
const SHARED_SUFFIXES: &[&str] = &[
    "amazonaws.com", "appspot.com", "azurewebsites.net", "azurestaticapps.net", "blogspot.com", "cloudapp.net", "cloudfront.net", "ddns.net",
    "duckdns.org", "dyndns.org", "dynv6.net", "firebaseapp.com", "fly.dev", "github.io", "gitlab.io", "glitch.me", "herokuapp.com", "hopto.org",
    "netlify.app", "ngrok-free.app", "ngrok.app", "ngrok.io", "no-ip.org", "no-ip.biz", "onrender.com", "pages.dev", "railway.app", "repl.co",
    "sytes.net", "trycloudflare.com", "vercel.app", "web.app", "workers.dev", "your-storageshare.de", "zapto.org",
];

/// Second-level labels that country registries hand out like top-level
/// domains (`example.co.uk`, `example.gob.mx`).
const CC_SECOND_LEVEL: &[&str] = &[
    "ac", "biz", "co", "com", "ed", "edu", "firm", "gen", "go", "gob", "gov", "gv", "ind", "info", "int", "lg", "ltd", "me", "med", "mil", "mod",
    "ne", "net", "nhs", "nic", "nom", "or", "org", "plc", "police", "res", "sch", "web",
];

/// The registrable part of a host name, roughly: its last two labels, or three
/// under a two-letter country code with a generic second level (`example.co.uk`).
/// `None` for IP addresses, single-label names and hosts under a shared
/// hosting suffix ([`SHARED_SUFFIXES`]), which are only ever their own site.
fn site(host: &str) -> Option<String> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return None;
    }
    if SHARED_SUFFIXES.iter().any(|s| host == *s || host.strip_suffix(s).is_some_and(|rest| rest.ends_with('.'))) {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    if n < 2 {
        return None;
    }
    let take = if n >= 3 && labels[n - 1].len() == 2 && CC_SECOND_LEVEL.contains(&labels[n - 2]) { 3 } else { 2 };
    if n < take {
        return None;
    }
    Some(labels[n - take..].join("."))
}

/// Whether an address is one an outside link must not reach: loopback,
/// private, link-local, unique-local, shared (CGNAT), multicast, unspecified.
pub fn is_internal_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_internal_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            let embedded = |hi: u16, lo: u16| IpAddr::V4(std::net::Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo)));
            // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) carry an IPv4
            // address that a gateway forwards to: judge that one.
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return is_internal_ip(embedded(s[6], s[7]));
            }
            if s[0] == 0x2002 {
                return is_internal_ip(embedded(s[1], s[2]));
            }
            // Teredo (2001::/32) ends in the client's IPv4, bit-inverted.
            if s[..2] == [0x2001, 0] {
                return is_internal_ip(embedded(!s[6], !s[7]));
            }
            // Local-use NAT64 (64:ff9b:1::/48) only reaches private networks.
            if s[..3] == [0x64, 0xff9b, 1] {
                return true;
            }
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Whether a URL names this computer or a private network directly
/// (`localhost`, or an internal IP literal).
pub fn is_internal_url(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => is_internal_ip(ip.into()),
        Some(url::Host::Ipv6(ip)) => is_internal_ip(ip.into()),
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        None => true,
    }
}

/// Whether `host` is this computer: `localhost`, 127.0.0.0/8 or ::1. Only
/// these may get a password over a connection without TLS.
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Refuses to sign in to `host` with no encryption unless it's this computer.
pub fn plaintext_allowed(host: &str, what: &str) -> Result<()> {
    if is_loopback_host(host) {
        return Ok(());
    }
    Err(anyhow::Error::new(satchel_api::ApiError::new(
        satchel_api::ErrorCode::InvalidInput,
        format!("{what} security is set to None, which would send your password unencrypted to {host}. Choose TLS or STARTTLS in the account settings."),
    )))
}

/// A resolver that refuses names resolving only to internal addresses, and
/// drops internal ones from mixed answers, so a link from an email can't
/// reach this computer or the local network through DNS.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.filter(|a| !is_internal_ip(a.ip())).collect();
            if addrs.is_empty() {
                return Err(format!("{host} is not a public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// A client for links that come from email (one-click unsubscribe): it only
/// reaches public hosts and follows redirects only to public https URLs.
pub fn public_http_client() -> Result<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(|a| {
        if a.previous().len() >= 5 {
            a.error("too many redirects")
        } else if a.url().scheme() != "https" || is_internal_url(a.url()) {
            a.error("redirected to a non-public or non-https address")
        } else {
            a.follow()
        }
    });
    // No proxy: a proxy would resolve names itself and skip the check above.
    Ok(http_builder(false).redirect(policy).dns_resolver(PublicOnly).no_proxy().build()?)
}

/// Map HTTP status codes to API error codes.
pub fn http_error(status: reqwest::StatusCode, body: &str, what: &str) -> anyhow::Error {
    use satchel_api::{ApiError, ErrorCode};
    let code = match status.as_u16() {
        401 => ErrorCode::Auth,
        403 if is_quota_body(body) => ErrorCode::RateLimited,
        403 => ErrorCode::Forbidden,
        404 | 410 => ErrorCode::NotFound,
        409 | 412 => ErrorCode::Conflict,
        429 => ErrorCode::RateLimited,
        400 | 422 => ErrorCode::InvalidInput,
        500..=599 => ErrorCode::Network,
        _ => ErrorCode::Internal,
    };
    let snippet: String = body.chars().take(300).collect();
    anyhow::Error::new(ApiError::new(code, format!("{what}: HTTP {status}: {snippet}")))
}

/// Whether a 403 body is Google's rate or quota limit rather than a refusal.
/// Google reports per-minute quotas as 403, not 429.
pub fn is_quota_body(body: &str) -> bool {
    ["rateLimitExceeded", "userRateLimitExceeded", "quotaExceeded", "Quota exceeded"].iter().any(|s| body.contains(s))
}

/// [`MaybeSent::classify`] for HTTP sends: a connection that never opened
/// sent nothing, so it stays retryable.
pub fn send_error(e: anyhow::Error) -> anyhow::Error {
    if e.chain().any(|c| c.downcast_ref::<reqwest::Error>().is_some_and(reqwest::Error::is_connect)) {
        return e;
    }
    satchel_core::provider::MaybeSent::classify(e)
}

/// Percent-encode a value for use as a URL path segment or query value
/// (everything but RFC 3986 unreserved characters).
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_trickling_server_is_cut_off() {
        use super::{TRICKLE_MIN_BYTES, TRICKLE_WINDOW, trickled};
        let t0 = tokio::time::Instant::now();
        let step = std::time::Duration::from_secs(170);
        // A byte every 170 s never trips the silence check, but fails here.
        let mut w = None;
        let mut failed = false;
        for i in 0..10 {
            if trickled(&mut w, 1, t0 + step * i).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        // Progress lines (~60 bytes every 10 s) on a long command are fine.
        let mut w = None;
        for i in 0..200 {
            trickled(&mut w, 60, t0 + std::time::Duration::from_secs(10 * i)).unwrap();
        }
        // So is a short burst followed by a long quiet wait for the next line.
        let mut w = None;
        trickled(&mut w, TRICKLE_MIN_BYTES, t0).unwrap();
        trickled(&mut w, 10, t0 + TRICKLE_WINDOW).unwrap();
    }

    #[test]
    fn credentials_stay_with_the_account_server() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        let base = u("https://caldav.example.com/dav/");
        assert!(super::credentials_allowed(&base, &u("https://caldav.example.com:443/other")));
        assert!(super::credentials_allowed(&u("https://caldav.icloud.com/"), &u("https://p42-caldav.icloud.com/123/calendars/")));
        assert!(!super::credentials_allowed(&base, &u("http://caldav.example.com/dav/")), "no downgrade");
        assert!(!super::credentials_allowed(&base, &u("https://evil.test/")));
        assert!(!super::credentials_allowed(&base, &u("https://caldav.example.com:8443/")));
        assert!(!super::credentials_allowed(&u("https://dav.example.co.uk/"), &u("https://evil.co.uk/")));
        assert!(super::credentials_allowed(&u("https://dav.example.co.uk/"), &u("https://p1.example.co.uk/")));
        assert!(!super::credentials_allowed(&u("https://10.0.0.1/"), &u("https://10.0.0.2/")));
        // Shared hosting: each customer is its own site.
        for (a, b) in [("https://alice.duckdns.org/", "https://mallory.duckdns.org/"), ("https://a.github.io/", "https://b.github.io/"), ("https://x.your-storageshare.de/", "https://y.your-storageshare.de/"), ("https://duckdns.org/", "https://evil.duckdns.org/")] {
            assert!(!super::credentials_allowed(&u(a), &u(b)), "{a} -> {b}");
        }
        assert!(super::credentials_allowed(&u("https://alice.duckdns.org/dav/"), &u("https://alice.duckdns.org/other")), "same origin still works");
        assert!(!super::credentials_allowed(&u("https://dav.example.gob.mx/"), &u("https://evil.gob.mx/")));
        assert!(!super::credentials_allowed(&u("https://dav.example.nhs.uk/"), &u("https://evil.nhs.uk/")));
        assert!(super::credentials_allowed(&u("https://www.google.com/calendar/dav/"), &u("https://apidata.google.com/x")));
        let plain = u("http://127.0.0.1:8080/dav/");
        assert!(super::credentials_allowed(&plain, &u("http://127.0.0.1:8080/x")));
        assert!(!super::credentials_allowed(&plain, &u("http://127.0.0.1:9090/x")));
    }

    #[test]
    fn internal_addresses() {
        let u = |s: &str| super::is_internal_url(&url::Url::parse(s).unwrap());
        for bad in ["https://127.0.0.1/", "https://localhost/", "https://a.localhost/", "https://10.1.2.3/", "https://192.168.0.1/", "https://172.16.5.5/", "https://169.254.169.254/", "https://[::1]/", "https://[fd00::1]/", "https://[fe80::1]/", "https://[::ffff:127.0.0.1]/", "https://0.0.0.0/", "https://100.64.0.1/"] {
            assert!(u(bad), "{bad}");
        }
        for bad in ["https://[64:ff9b::7f00:1]/", "https://[64:ff9b::a00:1]/", "https://[2002:c0a8:101::1]/", "https://[2002:7f00:1::]/", "https://[64:ff9b:1::1]/", "https://[2001:0:4136:e378:8000:63bf:f5ff:fffe]/"] {
            assert!(u(bad), "{bad}");
        }
        for good in ["https://example.com/", "https://8.8.8.8/", "https://[2606:4700::1111]/", "https://172.32.0.1/", "https://[64:ff9b::808:808]/", "https://[2002:808:808::1]/", "https://[2001:0:4136:e378:8000:63bf:f7f7:f7f7]/"] {
            assert!(!u(good), "{good}");
        }
    }

    #[tokio::test]
    async fn capped_reader_stops_at_the_limit() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..2 {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                // Chunked, so no Content-Length gives the size away up front.
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await;
                for _ in 0..4 {
                    let _ = s.write_all(b"400\r\n").await;
                    let _ = s.write_all(&[b'x'; 0x400]).await;
                    let _ = s.write_all(b"\r\n").await;
                }
                let _ = s.write_all(b"0\r\n\r\n").await;
            }
        });
        let http = super::http_client(false).unwrap();
        let url = format!("http://{addr}/");
        let small = http.get(&url).send().await.unwrap();
        assert!(super::body_capped(small, 1000).await.is_err());
        let big = http.get(&url).send().await.unwrap();
        assert_eq!(super::body_capped(big, 4096).await.unwrap().len(), 4096);
    }

    #[test]
    fn plaintext_only_to_this_computer() {
        for ok in ["localhost", "LOCALHOST", "127.0.0.1", "127.5.6.7", "::1", "[::1]"] {
            assert!(super::plaintext_allowed(ok, "IMAP").is_ok(), "{ok}");
        }
        for bad in ["mail.example.com", "10.0.0.1", "192.168.1.1", "localhost.example.com", "::2", "0.0.0.0"] {
            assert!(super::plaintext_allowed(bad, "IMAP").is_err(), "{bad}");
        }
    }

    #[test]
    fn encodes() {
        assert_eq!(super::enc("a b@c/d+é"), "a%20b%40c%2Fd%2B%C3%A9");
    }

    #[test]
    fn google_quota_403_is_rate_limited() {
        use satchel_api::{ApiError, ErrorCode};
        let quota = r#"{"error":{"code":403,"message":"Quota exceeded for quota metric 'Total Query Cost'","errors":[{"reason":"rateLimitExceeded"}]}}"#;
        let e = super::http_error(reqwest::StatusCode::FORBIDDEN, quota, "list labels");
        assert_eq!(e.downcast_ref::<ApiError>().unwrap().code, ErrorCode::RateLimited);
        let denied = r#"{"error":{"code":403,"errors":[{"reason":"insufficientPermissions"}]}}"#;
        let e = super::http_error(reqwest::StatusCode::FORBIDDEN, denied, "list labels");
        assert_eq!(e.downcast_ref::<ApiError>().unwrap().code, ErrorCode::Forbidden);
    }

    #[tokio::test]
    async fn a_silent_server_times_out() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client, mut server) = tokio::io::duplex(64);
        let mut timed = super::Timed::new(client, Some(std::time::Duration::from_millis(50)));
        server.write_all(b"* OK hi\r\n").await.unwrap();
        let mut buf = [0u8; 64];
        assert!(timed.read(&mut buf).await.unwrap() > 0, "data arrives normally");
        let err = timed.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        drop(server);
    }
}
