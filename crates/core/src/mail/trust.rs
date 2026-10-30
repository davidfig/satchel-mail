//! Which receiving server's Authentication-Results to believe. Anyone can
//! write that header into a message, so a pass only counts from the user's
//! own server, named by the authserv-id on the topmost header (RFC 8601).
//! Known providers have built-in ids; for other accounts the id is learned
//! from the account's recent mail. Until then no sender counts as verified.

use std::collections::HashMap;

use satchel_api::{Account, AccountConfig};
use satchel_mime::AuthServers;

use crate::engine::Engine;

/// Recent messages looked at to learn an account's authserv-id.
const LEARN_WINDOW: u32 = 200;
/// Messages that must carry the same authserv-id before it is trusted. It
/// must also be on more than half the recent messages read.
const LEARN_MIN: usize = 20;

const GOOGLE: &[&str] = &["mx.google.com"];
/// Microsoft 365 and Outlook.com mostly write no authserv-id at all.
const MICROSOFT: &[&str] = &["*.protection.outlook.com", "outlook.com"];
const FASTMAIL: &[&str] = &["*.messagingengine.com"];
const ICLOUD: &[&str] = &["*.icloud.com", "*.me.com"];

fn servers(ids: &[&str], no_id: bool) -> AuthServers {
    AuthServers { ids: ids.iter().map(|s| s.to_string()).collect(), no_id }
}

/// The receiving servers of a known provider, by account kind or mail host.
pub(crate) fn builtin_auth_servers(config: &AccountConfig) -> Option<AuthServers> {
    let host = match config {
        AccountConfig::Gmail { .. } => return Some(servers(GOOGLE, false)),
        AccountConfig::Graph { .. } => return Some(servers(MICROSOFT, true)),
        AccountConfig::Imap { imap_host, .. } => imap_host.as_str(),
        // https://api.fastmail.com/jmap/session -> api.fastmail.com
        AccountConfig::Jmap { session_url, .. } => {
            let rest = session_url.split_once("://").map_or(session_url.as_str(), |(_, r)| r);
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let host = authority.rsplit('@').next().unwrap_or("");
            host.split(':').next().unwrap_or("")
        }
    };
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let under = |domains: &[&str]| domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")));
    if under(&["gmail.com", "googlemail.com"]) {
        Some(servers(GOOGLE, false))
    } else if under(&["office365.com", "outlook.com", "hotmail.com", "live.com"]) {
        Some(servers(MICROSOFT, true))
    } else if under(&["fastmail.com"]) {
        Some(servers(FASTMAIL, false))
    } else if under(&["icloud.com", "me.com", "mac.com"]) {
        Some(servers(ICLOUD, false))
    } else {
        None
    }
}

/// The authserv-id of a raw message's topmost Authentication-Results
/// header; empty if it has none, or none we could ever trust.
fn top_authserv(raw: &[u8]) -> String {
    let find = |pat: &[u8]| raw.windows(pat.len()).position(|w| w == pat).map(|i| i + pat.len());
    let end = [find(b"\r\n\r\n"), find(b"\n\n")].into_iter().flatten().min().unwrap_or(raw.len());
    let header = satchel_mime::parse(&raw[..end]).and_then(|p| p.auth_results);
    header.and_then(|h| satchel_mime::authserv_id(&h)).unwrap_or_default()
}

/// Whether a trusted server positively verified the From domain (DMARC
/// pass, or a DKIM pass aligned with it): the test for trusting a From
/// address, shared by sender rules and image trust. Missing results don't
/// count, since anyone can put any address in From.
pub(crate) fn sender_verified_by(p: &satchel_mime::Parsed, trusted: &AuthServers) -> bool {
    let domain = p.from.as_ref().map(|f| satchel_mime::email_domain(&f.email)).unwrap_or_default();
    p.auth_results.as_deref().is_some_and(|a| satchel_mime::auth_verified(a, &domain, trusted))
}

impl Engine {
    /// Whether the account's own server verified this message's From domain.
    pub(crate) fn sender_verified(&self, account_id: &str, p: &satchel_mime::Parsed) -> bool {
        p.auth_results.is_some() && sender_verified_by(p, &self.auth_servers(account_id))
    }

    /// The servers whose Authentication-Results count for this account:
    /// the provider's own, or the one learned from recent mail. Empty
    /// (nothing trusted) until one is learned.
    pub(crate) fn auth_servers(&self, account_id: &str) -> AuthServers {
        let Ok(Some(acc)) = self.store.get_account(account_id) else { return AuthServers::default() };
        if let Some(b) = builtin_auth_servers(&acc.config) {
            return b;
        }
        self.learned_auth_server(&acc).map(|id| AuthServers { ids: vec![id], no_id: false }).unwrap_or_default()
    }

    /// The authserv-id on the topmost header of most of the account's recent
    /// mail from others. Each message's id is read from its stored copy once.
    fn learned_auth_server(&self, acc: &Account) -> Option<String> {
        let rows = self.store.recent_auth_servs(&acc.id, &acc.email, LEARN_WINDOW).ok()?;
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut read = 0;
        for (id, serv, hash) in rows {
            let serv = match serv {
                Some(s) => s,
                None => {
                    let Some(raw) = hash.and_then(|h| self.blobs.get(&h).ok().flatten()) else { continue };
                    let s = top_authserv(&raw);
                    if let Err(e) = self.store.set_auth_serv(&id, &s) {
                        tracing::debug!("recording authserv-id failed: {e:#}");
                    }
                    s
                }
            };
            read += 1;
            if !serv.is_empty() {
                *counts.entry(serv).or_default() += 1;
            }
        }
        let (id, n) = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
        (n >= LEARN_MIN && n * 2 > read).then_some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imap(host: &str) -> AccountConfig {
        serde_json::from_value(serde_json::json!({
            "kind": "imap", "email": "a@b.test", "display_name": "", "username": "a", "imap_host": host, "imap_port": 993,
            "imap_security": "tls", "smtp_host": host, "smtp_port": 465, "smtp_security": "tls"
        }))
        .unwrap()
    }

    #[test]
    fn known_providers() {
        assert_eq!(builtin_auth_servers(&imap("imap.gmail.com")).unwrap().ids, ["mx.google.com"]);
        assert!(builtin_auth_servers(&imap("outlook.office365.com")).unwrap().no_id);
        assert!(builtin_auth_servers(&imap("imap.mail.me.com")).unwrap().matches(Some("dmarc.icloud.com")));
        assert!(builtin_auth_servers(&imap("imap.fastmail.com")).unwrap().matches(Some("mx3.messagingengine.com")));
        assert!(builtin_auth_servers(&imap("mail.example.test")).is_none());
        assert!(builtin_auth_servers(&imap("gmail.com.evil.test")).is_none());
        let jmap: AccountConfig = serde_json::from_value(serde_json::json!({
            "kind": "jmap", "email": "a@b.test", "display_name": "", "username": "a", "session_url": "https://api.fastmail.com/jmap/session"
        }))
        .unwrap();
        assert!(builtin_auth_servers(&jmap).unwrap().matches(Some("mx1.messagingengine.com")));
    }

    #[test]
    fn reads_the_top_header_only() {
        let raw = b"Authentication-Results: MX.Mail.Test; dkim=none\r\nAuthentication-Results: mx.evil.test; dmarc=pass\r\nSubject: x\r\n\r\nAuthentication-Results: body.test; x=y\r\n";
        assert_eq!(top_authserv(raw), "mx.mail.test");
        assert_eq!(top_authserv(b"Subject: x\r\n\r\nhi"), "");
    }
}
