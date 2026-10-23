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
