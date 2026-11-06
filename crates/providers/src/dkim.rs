//! DKIM verification for accounts whose server's Authentication-Results
//! Satchel can't trust (0061). The core decides what a pass means; this
//! only reports which signing domains verified, using the system's DNS.

use std::sync::OnceLock;

use mail_auth::{AuthenticatedMessage, DkimOutput, DkimResult, MessageAuthenticator};

/// One resolver for the process; None if the system's DNS settings can't be read.
fn authenticator() -> Option<&'static MessageAuthenticator> {
    static AUTH: OnceLock<Option<MessageAuthenticator>> = OnceLock::new();
    AUTH.get_or_init(|| match MessageAuthenticator::new_system_conf() {
        Ok(a) => Some(a),
        Err(e) => {
            tracing::warn!("DKIM checks are off: can't read the system's DNS settings: {e}");
            None
        }
    })
    .as_ref()
}

/// The domains whose DKIM signatures on `raw` verify, lowercased. None if
/// none verified and a DNS lookup failed for a reason that may pass.
pub async fn passing_domains(raw: &[u8]) -> Option<Vec<String>> {
    let msg = AuthenticatedMessage::parse(raw)?;
    let outputs = authenticator()?.verify_dkim(&msg).await;
    domains(&msg, &outputs)
}

fn domains(msg: &AuthenticatedMessage, outputs: &[DkimOutput]) -> Option<Vec<String>> {
    // With two From headers, a signature can cover one while the reader
    // shows the other.
    if msg.headers.iter().filter(|(name, _)| name.eq_ignore_ascii_case(b"from")).count() != 1 {
        return Some(vec![]);
    }
    let mut out: Vec<String> = vec![];
    for o in outputs {
        let Some(sig) = o.signature() else { continue };
        // A signature that doesn't cover From doesn't vouch for it, and one
        // with l= covers only part of the body, so text can be added after it.
        let signs_from = sig.h.iter().any(|h| h.eq_ignore_ascii_case("from"));
        if *o.result() == DkimResult::Pass && signs_from && sig.l == 0 {
            let d = sig.d.trim_end_matches('.').to_ascii_lowercase();
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    let retry = outputs.iter().any(|o| matches!(o.result(), DkimResult::TempError(_)));
    (!out.is_empty() || !retry).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_mail_has_no_domains() {
        let raw = b"From: a@x.test\r\nTo: b@y.test\r\nSubject: hi\r\n\r\nhello\r\n";
        let msg = AuthenticatedMessage::parse(raw).unwrap();
        assert_eq!(domains(&msg, &[]), Some(vec![]));
    }

    #[test]
    fn two_from_headers_never_pass() {
        let raw = b"From: a@bank.test\r\nFrom: a@x.test\r\nSubject: hi\r\n\r\nhello\r\n";
        let msg = AuthenticatedMessage::parse(raw).unwrap();
        assert_eq!(domains(&msg, &[]), Some(vec![]));
    }
}
