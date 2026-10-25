//! Automatic contacts: who counts as a person you correspond with (pure rules).

use super::*;

/// Addresses that belong to machines, not people: no-reply senders, bounce
/// handlers, notification and newsletter senders, per-thread reply addresses.
pub fn is_machine_address(email: &str) -> bool {
    let email = email.trim().to_lowercase();
    let Some((local, domain)) = email.rsplit_once('@') else { return true };
    let squashed: String = local.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    const LOCAL: [&str; 11] =
        ["noreply", "donotreply", "mailerdaemon", "postmaster", "bounce", "notification", "newsletter", "unsubscribe", "autoconfirm", "autoreply", "mailbot"];
    if LOCAL.iter().any(|p| squashed.contains(p)) {
        return true;
    }
    // GitHub-style reply+<token>@ and long tokenized locals (VERP bounces).
    if local.starts_with("reply+") || local.starts_with("reply-") || local.starts_with("bounces") {
        return true;
    }
    if local.split(['+', '=']).skip(1).any(|t| t.len() >= 16) {
        return true;
    }
    domain.starts_with("noreply.") || domain.starts_with("reply.") || domain.starts_with("bounce.") || domain.starts_with("bounces.")
}

pub(crate) fn valid_address(e: &str) -> bool {
    let e = e.trim();
    e.len() > 3 && e.contains('@') && !e.contains(char::is_whitespace) && !e.starts_with('@') && !e.ends_with('@')
}

/// A display name worth keeping (not empty, not an address).
pub(crate) fn clean_name(n: Option<&str>, email: &str) -> Option<String> {
    let n = n?.trim().trim_matches(|c: char| c == '"' || c == '\'').trim();
    let n = n.split_whitespace().collect::<Vec<_>>().join(" ");
    (!n.is_empty() && !n.contains('@') && !n.eq_ignore_ascii_case(email)).then_some(n)
}

/// Turn raw evidence (address, name, date ms, named by themselves) into the
/// people the user corresponds with. Names people give themselves (in their
/// own From line) count more than names others give them.
pub fn correspondents(evidence: &[Evidence], own: &HashSet<String>, is_spam: impl Fn(&str) -> bool) -> Vec<Correspondent> {
    struct Acc {
        messages: u32,
        last: i64,
        names: HashMap<String, (u32, i64)>,
    }
    let mut by: HashMap<String, Acc> = HashMap::new();
    for (email, name, date, self_named) in evidence {
        let email = email.trim().to_lowercase();
        if !valid_address(&email) || own.contains(&email) || is_machine_address(&email) || is_spam(&email) {
            continue;
        }
        let a = by.entry(email.clone()).or_insert(Acc { messages: 0, last: 0, names: HashMap::new() });
        a.messages += 1;
        a.last = a.last.max(*date);
        if let Some(n) = clean_name(name.as_deref(), &email) {
            let v = a.names.entry(n).or_insert((0, 0));
            v.0 += if *self_named { 3 } else { 1 };
            v.1 = v.1.max(*date);
        }
    }
    let mut out: Vec<Correspondent> = by
        .into_iter()
        .map(|(email, a)| Correspondent {
            name: a.names.into_iter().max_by(|x, y| x.1.cmp(&y.1).then_with(|| y.0.cmp(&x.0))).map(|(n, _)| n),
            email,
            messages: a.messages,
            last_date: ms_to_dt(a.last),
        })
        .collect();
    out.sort_by(|a, b| a.email.cmp(&b.email));
    out
}
