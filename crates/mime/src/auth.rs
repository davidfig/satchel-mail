// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading the receiving server's Authentication-Results header (RFC 8601):
//! failures to warn about, and the positive, aligned pass that trust
//! decisions need. Anyone can write this header into a message, so a pass
//! only counts from a server the caller trusts (see [`AuthServers`]).

/// One result from an Authentication-Results header: `method=result` and
/// its properties (`header.d=example.com`), all lowercase.
#[derive(Debug, Clone, PartialEq)]
struct AuthResult {
    method: String,
    result: String,
    props: Vec<(String, String)>,
}

impl AuthResult {
    fn prop(&self, name: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// The receiving servers whose Authentication-Results headers count for
/// trust, by authserv-id. An entry `*.example.com` matches any subdomain of
/// example.com (not example.com itself); any other entry matches exactly.
/// Matching ignores case. `no_id` accepts a header without an authserv-id,
/// which is how Microsoft 365 and Outlook.com write theirs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthServers {
    pub ids: Vec<String>,
    pub no_id: bool,
}

impl AuthServers {
    /// Whether a header with this authserv-id (None: it has none) comes from a trusted server.
    pub fn matches(&self, id: Option<&str>) -> bool {
        let Some(id) = id.map(|i| i.trim().trim_end_matches('.').to_ascii_lowercase()) else { return self.no_id };
        !id.is_empty()
            && self.ids.iter().map(|t| t.to_ascii_lowercase()).any(|t| match t.strip_prefix("*.") {
                Some(parent) => id.ends_with(&format!(".{parent}")),
                None => id == t,
            })
    }
}

/// Split a header on `;` into its authserv-id and resinfos, dropping
/// (nested) comments and keeping quoted strings whole, so text a sender
/// controls (an envelope address echoed in a comment, a quoted reason)
/// can't pose as a result.
fn split_parts(header: &str) -> Vec<String> {
    let mut parts: Vec<String> = vec![String::new()];
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for c in header.chars() {
        let cur = parts.last_mut().expect("never empty");
        if escaped {
            if depth == 0 {
                cur.push(c);
            }
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted || depth > 0 => {
                escaped = true;
                if depth == 0 {
                    cur.push(c);
                }
            }
            '"' if depth == 0 => {
                quoted = !quoted;
                cur.push(c);
            }
            '(' if !quoted => depth += 1,
            ')' if !quoted && depth > 0 => depth -= 1,
            ';' if !quoted && depth == 0 => parts.push(String::new()),
            _ if depth == 0 => cur.push(if c.is_whitespace() { ' ' } else { c }),
            _ => {}
        }
    }
    parts
}

/// The authserv-id of an Authentication-Results header, lowercase: the
/// token before the first `;`, without its version. None when the header
/// starts with a result instead (Microsoft's form); empty when the id is
/// missing or isn't a host name, which no trusted server matches.
pub fn authserv_id(header: &str) -> Option<String> {
    let first = split_parts(header).into_iter().next().unwrap_or_default();
    if first.contains('=') {
        return None;
    }
    let id = first.split_whitespace().next().unwrap_or("").trim_end_matches('.').to_ascii_lowercase();
    let name_ok = id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    Some(if name_ok { id } else { String::new() })
}

/// Parse an Authentication-Results header (RFC 8601) into its results. Only
/// the `method=result` at the start of each resinfo counts. A header with
/// no authserv-id (Microsoft's form) starts with a resinfo.
fn parse_auth_results(header: &str) -> Vec<AuthResult> {
    let parts = split_parts(header);
    let skip = usize::from(parts.first().is_some_and(|p| !p.contains('=')));
    // Each resinfo is `method[/version] = result` then `ptype.prop = value` pairs.
    parts
        .iter()
        .skip(skip)
        .filter_map(|part| {
            let tokens = tokens(part);
            let mut it = tokens.into_iter();
            let (method, result) = it.next()?;
            let method = method.split('/').next().unwrap_or("").trim().to_ascii_lowercase();
            let result = result?.to_ascii_lowercase();
            let token_ok = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !token_ok(&method) || !token_ok(&result) {
                return None;
            }
            let props = it.filter_map(|(k, v)| Some((k.to_ascii_lowercase(), v?.trim_matches('"').to_ascii_lowercase()))).collect();
            Some(AuthResult { method, result, props })
        })
        .collect()
}

/// Split a comment-free resinfo into `key=value` pairs (spaces around `=`
/// allowed, quoted values kept whole). A value runs to the next space, so an
/// address like `a=header.d=x@y` stays one value. A bare word has no value.
fn tokens(s: &str) -> Vec<(String, Option<String>)> {
    let mut words: Vec<String> = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == ' ' && !quoted {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    // Rejoin `key = value` and `key= value` written with spaces.
    let mut joined: Vec<String> = vec![];
    for w in words {
        match joined.last_mut() {
            Some(last) if last.ends_with('=') || w.starts_with('=') => last.push_str(&w),
            _ => joined.push(w),
        }
    }
    joined
        .into_iter()
        .map(|w| match w.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (w, None),
        })
        .collect()
}

/// What an Authentication-Results header says went wrong, if anything.
/// Only failures are reported here, so it reads any header whoever wrote it:
/// a failure never grants trust. See `auth_verified` for the stricter test
/// used before trusting a sender.
pub fn auth_failure(auth_results: &str) -> Option<String> {
    let results = parse_auth_results(auth_results);
    let has = |method: &str, result: &str| results.iter().any(|r| r.method == method && r.result == result);
    let spf = results.iter().find(|r| r.method == "spf").map(|r| r.result.as_str());
    let dkim_ok = has("dkim", "pass");
    if has("dmarc", "fail") {
        return Some("failed DMARC".into());
    }
    match spf {
        Some("fail") if !dkim_ok => Some("failed SPF and has no valid DKIM signature".into()),
        Some("softfail") if !dkim_ok => Some("isn't authorized by the sender's domain (SPF softfail) and has no valid DKIM signature".into()),
        _ if has("dkim", "fail") && !dkim_ok && spf != Some("pass") => Some("has an invalid DKIM signature".into()),
        _ => None,
    }
}

/// Whether a trusted receiving server positively verified that the From
/// domain sent this message: the header's authserv-id is one of `trusted`,
/// and it says `dmarc=pass` (for this From domain, when it names one) or
/// `dkim=pass` with the From domain or a parent of it as the signing domain.
/// Missing results don't count, and any DMARC failure wins. This is the test
/// for trust decisions (remote images, sender rules). Pass only the topmost
/// header: the one the user's own server added last.
pub fn auth_verified(auth_results: &str, from_domain: &str, trusted: &AuthServers) -> bool {
    if !trusted.matches(authserv_id(auth_results).as_deref()) {
        return false;
    }
    let from = from_domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if from.is_empty() {
        return false;
    }
    let aligned = |d: &str| {
        let d = d.trim().trim_start_matches('@').trim_end_matches('.');
        !d.is_empty() && (from == d || from.ends_with(&format!(".{d}")))
    };
    let results = parse_auth_results(auth_results);
    if results.iter().any(|r| r.method == "dmarc" && r.result == "fail") {
        return false;
    }
    results.iter().any(|r| match (r.method.as_str(), r.result.as_str()) {
        ("dmarc", "pass") => r.prop("header.from").is_none_or(|d| d.trim_start_matches('@') == from),
        ("dkim", "pass") => r.prop("header.d").or_else(|| r.prop("header.i").map(|i| i.rsplit('@').next().unwrap_or(i))).is_some_and(aligned),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mx() -> AuthServers {
        AuthServers { ids: vec!["mx.x".into()], no_id: false }
    }

    #[test]
    fn auth_results() {
        assert!(auth_failure("mx.x; spf=fail smtp.mailfrom=a.test; dkim=fail; dmarc=fail header.from=a.test").unwrap().contains("DMARC"));
        assert!(auth_failure("mx.x; spf=softfail smtp.mailfrom=a.test; dkim=none").is_some());
        assert!(auth_failure("mx.x; spf=softfail; dkim=pass").is_none());
        assert!(auth_failure("mx.x; spf=pass; dkim=pass; dmarc=pass").is_none());
        // An envelope sender echoed in a comment can't pose as a result.
        let injected = "mx.x; spf=softfail (mx.x: domain of dkim=pass.dmarc=pass@bank.test does not designate 1.2.3.4) smtp.mailfrom=dkim=pass.dmarc=pass@bank.test; dmarc=fail header.from=bank.test";
        assert!(auth_failure(injected).unwrap().contains("DMARC"));
        assert!(!auth_verified(injected, "bank.test", &mx()));
        let quoted = r#"mx.x; spf=fail reason="dkim=pass; dmarc=pass" smtp.mailfrom=bank.test; dkim=none"#;
        assert!(auth_failure(quoted).unwrap().contains("SPF"));
        assert!(!auth_verified(quoted, "bank.test", &mx()));
        // Nested comments and spaces around `=`.
        assert!(auth_failure("mx.x (a (b; dmarc=pass) c); dmarc = fail header.from=a.test").is_some());
    }

    #[test]
    fn verified_senders() {
        assert!(auth_verified("mx.x; spf=pass smtp.mailfrom=bank.test; dkim=pass header.d=bank.test; dmarc=pass header.from=bank.test", "bank.test", &mx()));
        // DMARC pass for another From domain doesn't count.
        assert!(!auth_verified("mx.x; dmarc=pass header.from=evil.test", "bank.test", &mx()));
        // DKIM aligned to the From domain or a parent of it.
        assert!(auth_verified("mx.x; spf=none; dkim=pass header.d=bank.test", "mail.bank.test", &mx()));
        assert!(auth_verified("mx.x; dkim=pass header.i=@bank.test", "bank.test", &mx()));
        assert!(!auth_verified("mx.x; dkim=pass header.d=evilbank.test", "bank.test", &mx()));
        assert!(!auth_verified("mx.x; dkim=pass header.d=mail.bank.test", "bank.test", &mx()));
        assert!(!auth_verified("mx.x; dkim=pass header.d=sendgrid.test; spf=pass smtp.mailfrom=bank.test", "bank.test", &mx()));
        // Missing results, SPF alone, and "none" aren't verification.
        assert!(!auth_verified("", "bank.test", &mx()));
        assert!(!auth_verified("mx.x; none", "bank.test", &mx()));
        assert!(!auth_verified("mx.x; spf=pass smtp.mailfrom=bank.test", "bank.test", &mx()));
        assert!(!auth_verified("mx.x; dkim=pass header.d=bank.test; dmarc=fail header.from=bank.test", "bank.test", &mx()));
        assert!(!auth_verified("mx.x; dmarc=pass", "", &mx()));
        // A local part with `=` can't smuggle in a second property.
        assert!(!auth_verified("mx.x; dkim=pass header.i=x=header.d=bank.test@evil.test header.d=evil.test", "bank.test", &mx()));
    }

    #[test]
    fn authserv_ids() {
        assert_eq!(authserv_id("mx.google.com; dkim=pass header.d=a.test").as_deref(), Some("mx.google.com"));
        // The version and comments aren't part of it; case is folded.
        assert_eq!(authserv_id("MX3.MessagingEngine.com 1 (a; b); spf=pass").as_deref(), Some("mx3.messagingengine.com"));
        // Microsoft writes no authserv-id.
        assert_eq!(authserv_id("spf=pass (sender IP is 1.2.3.4) smtp.mailfrom=a.test; dkim=pass header.d=a.test;dmarc=pass action=none header.from=a.test"), None);
        assert_eq!(authserv_id("*.evil; dmarc=pass").as_deref(), Some(""));
        assert_eq!(authserv_id("; dmarc=pass").as_deref(), Some(""));
    }

    #[test]
    fn trusted_servers() {
        let google = AuthServers { ids: vec!["mx.google.com".into()], no_id: false };
        let ms = AuthServers { ids: vec!["*.protection.outlook.com".into(), "outlook.com".into()], no_id: true };
        let pass = "dkim=pass header.d=bank.test; dmarc=pass header.from=bank.test";
        assert!(auth_verified(&format!("mx.google.com; {pass}"), "bank.test", &google));
        assert!(auth_verified(&format!("MX.Google.COM; {pass}"), "bank.test", &google));
        // The right result from the wrong server, or from no named server.
        assert!(!auth_verified(&format!("mx.evil.test; {pass}"), "bank.test", &google));
        assert!(!auth_verified(&format!("evil.mx.google.com; {pass}"), "bank.test", &google));
        assert!(!auth_verified(pass, "bank.test", &google));
        assert!(!auth_verified(&format!("mx.google.com; {pass}"), "bank.test", &AuthServers::default()));
        // Wildcards match subdomains only; Microsoft's header has no id.
        assert!(auth_verified(&format!("spf=pass (sender IP is 1.2.3.4) smtp.mailfrom=bank.test; {pass}"), "bank.test", &ms));
        assert!(auth_verified(&format!("eur01.prod.protection.outlook.com; {pass}"), "bank.test", &ms));
        assert!(!auth_verified(&format!("protection.outlook.com; {pass}"), "bank.test", &ms));
        assert!(!auth_verified(&format!("protection.outlook.com.evil.test; {pass}"), "bank.test", &ms));
        // A Microsoft-form header's first result is read, so its failures warn.
        assert!(auth_failure("spf=fail (sender IP is 1.2.3.4) smtp.mailfrom=bank.test; dkim=none (message not signed) header.d=none").is_some());
    }

    #[test]
    fn parse_keeps_only_the_topmost_header() {
        // The receiving server adds its header above any the sender wrote.
        let raw = b"Authentication-Results: mx.google.com; dkim=none; dmarc=none header.from=bank.test\r\n\
            Received: from evil.test\r\n\
            Authentication-Results: mx.google.com; dkim=pass header.d=bank.test; dmarc=pass header.from=bank.test\r\n\
            From: a@bank.test\r\nSubject: x\r\n\r\nhi\r\n";
        let top = crate::parse(raw).unwrap().auth_results.unwrap();
        assert!(top.contains("dmarc=none"), "{top}");
        assert!(!auth_verified(&top, "bank.test", &AuthServers { ids: vec!["mx.google.com".into()], no_id: false }));
    }
}
