//! Sending through SMTP: the account's server, or a send-as address's own (0072).

use super::*;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use satchel_core::provider::SmtpRelay;

type Transport = AsyncSmtpTransport<Tokio1Executor>;

/// Where a send goes: the account's SMTP or a send-as address's server.
struct Route<'a> {
    host: &'a str,
    port: u16,
    security: Security,
    username: &'a str,
    password: String,
}

fn transport(r: &Route<'_>, allow_invalid_certs: bool) -> Result<Transport> {
    if r.security == Security::None {
        crate::net::plaintext_allowed(r.host, "SMTP")?;
    }
    let tls = TlsParameters::builder(r.host.to_string())
        .dangerous_accept_invalid_certs(allow_invalid_certs)
        .dangerous_accept_invalid_hostnames(allow_invalid_certs)
        .build_rustls()?;
    let b = Transport::builder_dangerous(r.host)
        .port(r.port)
        .timeout(Some(Duration::from_secs(60)))
        .credentials(Credentials::new(r.username.to_string(), r.password.clone()));
    Ok(match r.security {
        Security::Tls => b.tls(Tls::Wrapper(tls)),
        Security::StartTls => b.tls(Tls::Required(tls)),
        Security::None => b.tls(Tls::None),
    }
    .build())
}

/// Connect and sign in. Failing here means nothing was sent, so the op can
/// retry (offline, server down).
async fn connect(t: &Transport, host: &str) -> Result<()> {
    let ok = t.test_connection().await.map_err(|e| {
        let code = if e.is_permanent() { ErrorCode::Auth } else { ErrorCode::Network };
        anyhow::Error::new(ApiError::new(code, format!("SMTP ({host}): {e}")))
    })?;
    if !ok {
        return Err(anyhow::Error::new(ApiError::new(ErrorCode::Network, format!("SMTP ({host}): the server didn't answer"))));
    }
    Ok(())
}

/// Check a send-as address's server settings and password.
pub async fn test_relay(relay: &SmtpRelay, allow_invalid_certs: bool) -> Result<()> {
    let s = &relay.server;
    let r = Route { host: &s.host, port: s.port, security: s.security, username: &s.username, password: relay.password.clone() };
    connect(&transport(&r, allow_invalid_certs)?, &s.host).await
}

impl ImapProvider {
    fn route<'a>(&'a self, relay: Option<&'a SmtpRelay>) -> Result<Route<'a>> {
        Ok(match relay {
            Some(x) => Route { host: &x.server.host, port: x.server.port, security: x.server.security, username: &x.server.username, password: x.password.clone() },
            None => {
                let c = &self.cfg;
                Route { host: &c.smtp_host, port: c.smtp_port, security: c.smtp_security, username: &c.username, password: self.password()? }
            }
        })
    }

    pub(super) async fn smtp_send(&self, raw: &[u8], from: &str, rcpts: &[String], relay: Option<&SmtpRelay>) -> Result<()> {
        use lettre::address::Envelope;
        let r = self.route(relay)?;
        let t = transport(&r, self.cfg.allow_invalid_certs)?;
        let env = Envelope::new(
            Some(from.parse().map_err(|e| anyhow!("bad from address {from}: {e}"))?),
            rcpts.iter().map(|r| r.parse().map_err(|e| anyhow!("bad recipient {r}: {e}"))).collect::<Result<Vec<_>>>()?,
        )?;
        connect(&t, r.host).await?;
        t.send_raw(&env, raw).await.map_err(|e| {
            let msg = format!("SMTP ({}): {e}", r.host);
            // A reply code means the server refused it; anything else (a
            // dropped connection, no reply after the message body) may have
            // been accepted.
            if e.is_permanent() {
                anyhow::Error::new(ApiError::new(ErrorCode::InvalidInput, msg))
            } else if e.is_transient() {
                anyhow::Error::new(ApiError::new(ErrorCode::Network, msg))
            } else {
                anyhow::Error::new(MaybeSent(anyhow!(msg)))
            }
        })?;
        Ok(())
    }

    /// Whether the server a send went through files its own copy in Sent.
    pub(super) fn server_saves_sent(&self, relay: Option<&SmtpRelay>) -> bool {
        let h = relay.map_or(&self.cfg.smtp_host, |r| &r.server.host).to_lowercase();
        h.ends_with("gmail.com") || h.ends_with("googlemail.com") || h.contains("office365.com") || h.contains("outlook.com")
    }
}
