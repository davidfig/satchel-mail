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
pub struct Timed<S> {
    inner: S,
    stall: Option<Duration>,
    read_deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    write_deadline: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl<S> Timed<S> {
    pub fn new(inner: S, stall: Option<Duration>) -> Self {
        Self { inner, stall, read_deadline: None, write_deadline: None }
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
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Pending => stalled(&mut this.read_deadline, this.stall, cx),
            ready => {
                this.read_deadline = None;
                ready
            }
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Timed<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
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
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("SatchelMail/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(Duration::from_secs(120));
    if allow_invalid {
        b = b.tls_danger_accept_invalid_certs(true);
    }
    Ok(b.build()?)
}

/// Map HTTP status codes to API error codes.
pub fn http_error(status: reqwest::StatusCode, body: &str, what: &str) -> anyhow::Error {
    use satchel_api::{ApiError, ErrorCode};
    let code = match status.as_u16() {
        401 => ErrorCode::Auth,
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
    fn encodes() {
        assert_eq!(super::enc("a b@c/d+é"), "a%20b%40c%2Fd%2B%C3%A9");
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
