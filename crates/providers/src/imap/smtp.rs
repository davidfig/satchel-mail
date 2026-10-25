//! Sending through SMTP.

use super::*;

impl ImapProvider {
    pub(super) async fn smtp_send(&self, raw: &[u8], from: &str, rcpts: &[String]) -> Result<()> {
        use lettre::address::Envelope;
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::transport::smtp::client::{Tls, TlsParameters};
        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
        let c = &self.cfg;
        let tls = TlsParameters::builder(c.smtp_host.clone())
            .dangerous_accept_invalid_certs(c.allow_invalid_certs)
            .dangerous_accept_invalid_hostnames(c.allow_invalid_certs)
            .build_rustls()?;
        let mut b = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(c.smtp_host.clone())
            .port(c.smtp_port)
            .timeout(Some(Duration::from_secs(60)))
            .credentials(Credentials::new(c.username.clone(), self.password()?));
        b = match c.smtp_security {
            Security::Tls => b.tls(Tls::Wrapper(tls)),
            Security::StartTls => b.tls(Tls::Required(tls)),
            Security::None => b.tls(Tls::None),
        };
        let t = b.build();
        let env = Envelope::new(
            Some(from.parse().map_err(|e| anyhow!("bad from address {from}: {e}"))?),
            rcpts.iter().map(|r| r.parse().map_err(|e| anyhow!("bad recipient {r}: {e}"))).collect::<Result<Vec<_>>>()?,
        )?;
        // Connect and sign in first: failing here means nothing was sent, so
        // the op can retry (offline, server down).
        let ok = t.test_connection().await.map_err(|e| {
            let code = if e.is_permanent() { ErrorCode::Auth } else { ErrorCode::Network };
            anyhow::Error::new(ApiError::new(code, format!("SMTP: {e}")))
        })?;
        if !ok {
            return Err(anyhow::Error::new(ApiError::new(ErrorCode::Network, "SMTP: the server didn't answer")));
        }
        t.send_raw(&env, raw).await.map_err(|e| {
            let msg = format!("SMTP: {e}");
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

    pub(super) fn server_saves_sent(&self) -> bool {
        let h = self.cfg.smtp_host.to_lowercase();
        h.ends_with("gmail.com") || h.ends_with("googlemail.com") || h.contains("office365.com") || h.contains("outlook.com")
    }
}
