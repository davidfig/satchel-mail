//! Send-as addresses: setting them, choosing From, and picking the SMTP
//! server a send goes through (0072).

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use satchel_api::*;
use serde_json::json;

use crate::engine::Engine;
use crate::error::{err, invalid, not_found};
use crate::provider::SmtpRelay;
use crate::secrets::send_as_key;

impl Engine {
    pub(crate) async fn set_send_as(&self, account_id: &str, list: Vec<SendAs>, mut creds: BTreeMap<String, Credential>) -> Result<Account> {
        let acc = self.store.get_account(account_id)?.ok_or_else(|| not_found("account"))?;
        let list = clean_send_as(&acc, list)?;
        creds = creds.into_iter().map(|(k, v)| (k.trim().to_lowercase(), v)).collect();
        let allow_invalid_certs = matches!(acc.config, AccountConfig::Imap { allow_invalid_certs: true, .. });
        // Check every new or changed server before saving anything.
        let mut save = vec![];
        for a in &list {
            let Some(server) = &a.smtp else { continue };
            let key = a.email.to_lowercase();
            let typed = creds.remove(&key);
            let before = acc.send_as.iter().find(|b| b.email.eq_ignore_ascii_case(&a.email)).and_then(|b| b.smtp.as_ref());
            if typed.is_none() && before == Some(server) {
                continue;
            }
            let cred = match typed {
                Some(c) => c,
                None => self
                    .secrets
                    .get(&send_as_key(account_id, &a.email))?
                    .ok_or_else(|| invalid(format!("enter the SMTP password for {}", a.email)))?,
            };
            let Credential::Password { password } = &cred else {
                return Err(invalid(format!("the SMTP sign-in for {} must be a password", a.email)));
            };
            let relay = SmtpRelay { server: server.clone(), password: password.clone() };
            self.factory
                .test_smtp(&relay, allow_invalid_certs)
                .await
                .map_err(|e| err(ErrorCode::Auth, format!("{}: {:#}", a.email, e)))?;
            save.push((a.email.clone(), cred));
        }
        self.store.set_send_as(account_id, &list)?;
        for (email, cred) in save {
            self.secrets.set(&send_as_key(account_id, &email), &cred)?;
        }
        // Passwords of servers no longer used.
        for old in acc.send_as.iter().filter(|o| o.smtp.is_some()) {
            if !list.iter().any(|a| a.smtp.is_some() && a.email.eq_ignore_ascii_case(&old.email))
                && let Err(e) = self.secrets.delete(&send_as_key(account_id, &old.email))
            {
                tracing::warn!(account = %acc.email, "deleting the SMTP password for {}: {e:#}", old.email);
            }
        }
        self.store.audit("user", "send_as_changed", &json!({ "account": acc.email, "send_as": list.iter().map(|a| &a.email).collect::<Vec<_>>() }))?;
        self.emit(CoreEvent::AccountsChanged);
        self.store.get_account(account_id)?.ok_or_else(|| not_found("account"))
    }

    /// The server a send from `from` goes through, when its send-as address
    /// has one, and whether `from` is a send-as address at all.
    pub(crate) fn send_route(&self, acc: &Account, from: &str) -> Result<(bool, Option<SmtpRelay>)> {
        if from.is_empty() || from.eq_ignore_ascii_case(&acc.email) {
            return Ok((false, None));
        }
        let a = find(acc, from).ok_or_else(|| invalid(format!("{from} is no longer one of {}'s send-as addresses", acc.email)))?;
        let Some(server) = &a.smtp else { return Ok((true, None)) };
        let password = match self.secrets.get(&send_as_key(&acc.id, &a.email))? {
            Some(Credential::Password { password }) => password,
            _ => return Err(err(ErrorCode::Auth, format!("no SMTP password is saved for {}; enter it in Settings → Accounts", a.email))),
        };
        Ok((true, Some(SmtpRelay { server: server.clone(), password })))
    }
}

/// Check and tidy a list of send-as addresses for an account.
pub(crate) fn clean_send_as(acc: &Account, list: Vec<SendAs>) -> Result<Vec<SendAs>> {
    if acc.kind == ProviderKind::Graph && !list.is_empty() {
        return Err(invalid("Microsoft 365 accounts can't send as other addresses in Satchel"));
    }
    let mut seen = HashSet::new();
    let mut out = vec![];
    for mut a in list {
        a.email = a.email.trim().to_string();
        a.display_name = a.display_name.trim().to_string();
        let ok = a.email.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.ends_with('.'))
            && !a.email.chars().any(|c| c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | '"'));
        if !ok {
            return Err(invalid(format!("“{}” isn't an email address", a.email)));
        }
        if a.email.eq_ignore_ascii_case(&acc.email) {
            return Err(invalid(format!("{} is the account's own address", a.email)));
        }
        if !seen.insert(a.email.to_lowercase()) {
            return Err(invalid(format!("{} is listed twice", a.email)));
        }
        if let Some(s) = &mut a.smtp {
            if acc.kind != ProviderKind::Imap {
                return Err(invalid("only IMAP accounts can send through another SMTP server"));
            }
            s.host = s.host.trim().to_string();
            s.username = s.username.trim().to_string();
            if s.host.is_empty() || s.port == 0 {
                return Err(invalid(format!("enter the SMTP server and port for {}", a.email)));
            }
            if s.username.is_empty() {
                return Err(invalid(format!("enter the SMTP username for {}", a.email)));
            }
        }
        out.push(a);
    }
    Ok(out)
}

fn find<'a>(acc: &'a Account, email: &str) -> Option<&'a SendAs> {
    acc.send_as.iter().find(|a| a.email.eq_ignore_ascii_case(email.trim()))
}

/// The From for a message from `acc`: the account's own address, or the
/// send-as address `wanted` names. Anything else is refused, so a draft
/// (or an agent's proposal) can't invent a sender.
pub(crate) fn sender(acc: &Account, wanted: Option<&Address>) -> Result<Address> {
    let own = Address { name: Some(acc.display_name.clone()).filter(|s| !s.is_empty()), email: acc.email.clone() };
    let Some(w) = wanted.filter(|w| !w.email.trim().is_empty() && !w.email.trim().eq_ignore_ascii_case(&acc.email)) else {
        return Ok(own);
    };
    let a = find(acc, &w.email).ok_or_else(|| invalid(format!("{} isn't one of {}'s addresses; add it under send-as addresses in Settings → Accounts", w.email.trim(), acc.email)))?;
    let name = if a.display_name.is_empty() { own.name } else { Some(a.display_name.clone()) };
    Ok(Address { name, email: a.email.clone() })
}

/// Every address of the account, lowercased: its own and its send-as ones.
pub(crate) fn own_addresses(acc: &Account) -> HashSet<String> {
    std::iter::once(&acc.email).chain(acc.send_as.iter().map(|a| &a.email)).map(|e| e.to_lowercase()).collect()
}

/// The From a reply defaults to: the send-as address the original was sent
/// to (To, then Cc, then the delivery headers), or the one it was sent from
/// when it's the user's own message. None: the account's own address.
pub(crate) fn reply_from(acc: &Account, from: Option<&Address>, to: &[Address], cc: &[Address], delivered_to: &[String]) -> Option<Address> {
    let pick = |email: &str| find(acc, email).map(|a| Address { name: None, email: a.email.clone() });
    from.and_then(|f| pick(&f.email))
        .or_else(|| to.iter().chain(cc).find_map(|a| pick(&a.email)))
        .or_else(|| delivered_to.iter().find_map(|e| pick(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(kind: ProviderKind) -> Account {
        // Only `kind` matters to these rules.
        let config = AccountConfig::Gmail { email: "me@gmail.test".into(), display_name: "Me".into() };
        Account {
            id: "a1".into(),
            kind,
            email: "me@gmail.test".into(),
            display_name: "Me".into(),
            config,
            sync: SyncStatus::default(),
            color: String::new(),
            contacts_need_reconnect: false,
            send_as: vec![
                SendAs { email: "hello@satchel.test".into(), display_name: "Satchel".into(), smtp: None },
                SendAs { email: "david@satchel.test".into(), display_name: String::new(), smtp: None },
            ],
        }
    }

    fn addr(e: &str) -> Address {
        Address { name: None, email: e.into() }
    }

    #[test]
    fn from_is_the_account_or_one_of_its_send_as_addresses() {
        let acc = account(ProviderKind::Gmail);
        assert_eq!(sender(&acc, None).unwrap().email, "me@gmail.test");
        assert_eq!(sender(&acc, Some(&addr("ME@gmail.test"))).unwrap().email, "me@gmail.test");
        let h = sender(&acc, Some(&addr(" Hello@Satchel.test "))).unwrap();
        assert_eq!((h.email.as_str(), h.name.as_deref()), ("hello@satchel.test", Some("Satchel")));
        // No display name of its own: the account's.
        assert_eq!(sender(&acc, Some(&addr("david@satchel.test"))).unwrap().name.as_deref(), Some("Me"));
        // A name in the draft doesn't override the configured one.
        let named = Address { name: Some("Someone Else".into()), email: "hello@satchel.test".into() };
        assert_eq!(sender(&acc, Some(&named)).unwrap().name.as_deref(), Some("Satchel"));
        assert!(sender(&acc, Some(&addr("ceo@bank.test"))).is_err());
    }

    #[test]
    fn replies_default_to_the_address_the_mail_was_sent_to() {
        let acc = account(ProviderKind::Gmail);
        let bob = addr("bob@x.test");
        // To wins over Cc and the delivery headers.
        let r = reply_from(&acc, Some(&bob), &[addr("HELLO@satchel.test")], &[addr("david@satchel.test")], &["david@satchel.test".into()]);
        assert_eq!(r.unwrap().email, "hello@satchel.test");
        assert_eq!(reply_from(&acc, Some(&bob), &[addr("team@x.test")], &[addr("david@satchel.test")], &[]).unwrap().email, "david@satchel.test");
        // Bcc'd or via a list: only the delivery headers name it.
        assert_eq!(reply_from(&acc, Some(&bob), &[addr("list@x.test")], &[], &["me@gmail.test".into(), "hello@satchel.test".into()]).unwrap().email, "hello@satchel.test");
        // Sent to the account's own address: the account's.
        assert!(reply_from(&acc, Some(&bob), &[addr("me@gmail.test")], &[], &[]).is_none());
        // Replying to your own message keeps the address it was sent from.
        assert_eq!(reply_from(&acc, Some(&addr("david@satchel.test")), &[addr("hello@satchel.test")], &[], &[]).unwrap().email, "david@satchel.test");
    }

    #[test]
    fn send_as_lists_are_checked() {
        let acc = account(ProviderKind::Gmail);
        let a = |e: &str| SendAs { email: e.into(), ..Default::default() };
        let ok = clean_send_as(&acc, vec![SendAs { email: " hi@x.test ".into(), display_name: " Hi ".into(), smtp: None }]).unwrap();
        assert_eq!((ok[0].email.as_str(), ok[0].display_name.as_str()), ("hi@x.test", "Hi"));
        assert!(clean_send_as(&acc, vec![a("nope")]).is_err());
        assert!(clean_send_as(&acc, vec![a("a@b.test, c@d.test")]).is_err());
        assert!(clean_send_as(&acc, vec![a("Me@gmail.test")]).is_err());
        assert!(clean_send_as(&acc, vec![a("x@y.test"), a("X@y.test")]).is_err());
        // Own SMTP servers are for IMAP accounts.
        let smtp = SmtpServer { host: "smtp.x.test".into(), port: 465, security: Security::Tls, username: "api_token".into() };
        assert!(clean_send_as(&acc, vec![SendAs { email: "x@y.test".into(), display_name: String::new(), smtp: Some(smtp) }]).is_err());
        // Microsoft 365 has none.
        assert!(clean_send_as(&account(ProviderKind::Graph), vec![a("x@y.test")]).is_err());
        assert!(clean_send_as(&account(ProviderKind::Graph), vec![]).is_ok());
    }
}
