//! Signals about whether a message is safe: unsubscribe headers, sender
//! authentication results, deceptive links and risky attachments. These only
//! read the message; deciding what to show is the core's job.

use satchel_api::Unsubscribe;

/// The registrable part of a host, approximately: the last two labels, or
/// three for two-letter country domains with a short second level
/// ("example.co.uk"). Good enough to compare where two links point.
pub fn base_domain(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    if n <= 2 {
        return labels.join(".");
    }
    let tld = labels[n - 1];
    let sld = labels[n - 2];
    let keep = if tld.len() == 2 && (sld.len() <= 3 || matches!(sld, "gov" | "ac" | "co" | "com" | "net" | "org")) { 3 } else { 2 };
    labels[n - keep..].join(".")
}

/// Domain part of an email address.
pub fn email_domain(email: &str) -> String {
    email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default()
}

/// Attachments that can run code or imitate a web page when opened.
pub fn is_risky_attachment(filename: &str, mime_type: &str) -> bool {
    const EXT: &[&str] = &[
        "html", "htm", "shtml", "xhtml", "svg", "hta", "exe", "scr", "com", "bat", "cmd", "pif", "msi", "msix", "dll", "js", "jse", "vbs",
        "vbe", "wsf", "ps1", "jar", "lnk", "iso", "img", "vhd", "app", "dmg", "pkg", "command", "sh", "docm", "xlsm", "pptm", "xlam", "one",
    ];
    let name = filename.trim().to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if EXT.contains(&ext) {
        return true;
    }
    let m = mime_type.to_ascii_lowercase();
    m == "text/html" || m == "application/x-msdownload" || m == "application/x-ms-installer" || m == "application/javascript"
}

/// Parse List-Unsubscribe (and List-Unsubscribe-Post for one-click).
pub fn parse_unsubscribe(list_unsubscribe: &str, post: Option<&str>) -> Option<Unsubscribe> {
    let mut u = Unsubscribe::default();
    for part in list_unsubscribe.split(',') {
        let v = part.trim().trim_start_matches('<').trim_end_matches('>').trim();
        let lower = v.to_ascii_lowercase();
        if lower.starts_with("https://") && u.url.is_none() {
            u.url = Some(v.to_string());
        } else if lower.starts_with("mailto:") && u.mailto.is_none() {
            u.mailto = Some(v.to_string());
        }
    }
    // RFC 8058: one-click needs the exact POST body and an https URL.
    u.one_click = u.url.is_some() && post.is_some_and(|p| p.split_whitespace().collect::<String>().eq_ignore_ascii_case("List-Unsubscribe=One-Click"));
    (u.url.is_some() || u.mailto.is_some()).then_some(u)
}

/// A mailto: URL as (address, subject, body).
pub fn parse_mailto(url: &str) -> Option<(String, String, String)> {
    let rest = url.get(..7).filter(|s| s.eq_ignore_ascii_case("mailto:")).map(|_| &url[7..])?;
    let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
    let addr = percent_decode(addr);
    if !addr.contains('@') || addr.contains(char::is_whitespace) {
        return None;
    }
    let (mut subject, mut body) = (String::new(), String::new());
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k.to_ascii_lowercase().as_str() {
            "subject" => subject = percent_decode(v),
            "body" => body = percent_decode(v),
            _ => {}
        }
    }
    Some((addr, subject, body))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the receiving server's Authentication-Results header says went
/// wrong, if anything. Only failures are reported: a sender can forge a
/// "pass" header when the receiving server adds none, so a pass proves nothing.
pub fn auth_failure(auth_results: &str) -> Option<String> {
    let a = auth_results.to_ascii_lowercase();
    let result = |method: &str| -> Option<String> {
        let i = a.find(&format!("{method}="))?;
        Some(a[i + method.len() + 1..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect())
    };
    let (spf, dkim, dmarc) = (result("spf"), result("dkim"), result("dmarc"));
    let dkim_ok = dkim.as_deref() == Some("pass");
    if dmarc.as_deref() == Some("fail") {
        return Some("failed DMARC".into());
    }
    match spf.as_deref() {
        Some("fail") if !dkim_ok => Some("failed SPF and has no valid DKIM signature".into()),
        Some("softfail") if !dkim_ok => Some("isn't authorized by the sender's domain (SPF softfail) and has no valid DKIM signature".into()),
        _ if dkim.as_deref() == Some("fail") && spf.as_deref() != Some("pass") => Some("has an invalid DKIM signature".into()),
        _ => None,
    }
}

/// Links whose visible text names one domain but whose target is another,
/// as (shown host, actual host).
pub fn deceptive_links(html: &str) -> Vec<(String, String)> {
    let lower = html.to_ascii_lowercase();
    let mut out: Vec<(String, String)> = vec![];
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("<a") {
        let start = pos + i;
        pos = start + 2;
        if !lower[pos..].starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let Some(tag_end) = lower[start..].find('>').map(|j| start + j) else { break };
        let Some(close) = lower[tag_end..].find("</a").map(|j| tag_end + j) else { break };
        let href = attr(&html[start..tag_end], "href");
        let text = decode_entities(&strip_tags(&html[tag_end + 1..close]));
        pos = close;
        let (Some(href), Some(shown)) = (href, shown_host(&text)) else { continue };
        let Some(actual) = url_host(&href) else { continue };
        if base_domain(&shown) != base_domain(&actual) && !out.iter().any(|(s, a)| *s == shown && *a == actual) {
            out.push((shown, actual));
        }
    }
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = lower[..at].ends_with(|c: char| c.is_whitespace());
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let vstart = tag.len() - rest.len() + 1;
        let v = tag[vstart..].trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or("").to_string(),
            _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
        });
    }
    None
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&#46;", ".").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// The host a link's text claims to go to, if the text looks like a URL or domain.
fn shown_host(text: &str) -> Option<String> {
    let t = text.trim();
    if t.contains(char::is_whitespace) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or(lower.strip_prefix("http://")).unwrap_or(&lower);
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    let valid = labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels.last().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    valid.then(|| host.to_string())
}

fn url_host(href: &str) -> Option<String> {
    let lower = href.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or(lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains() {
        assert_eq!(base_domain("www.paypal.com"), "paypal.com");
        assert_eq!(base_domain("mail.example.co.uk"), "example.co.uk");
        assert_eq!(base_domain("a.b.example.de"), "example.de");
    }

    #[test]
    fn unsubscribe_headers() {
        let u = parse_unsubscribe("<https://x.test/u?a=1>, <mailto:unsub@x.test?subject=stop>", Some("List-Unsubscribe=One-Click")).unwrap();
        assert_eq!(u.url.as_deref(), Some("https://x.test/u?a=1"));
        assert_eq!(u.mailto.as_deref(), Some("mailto:unsub@x.test?subject=stop"));
        assert!(u.one_click);
        // http (not https) URLs are never used.
        let u = parse_unsubscribe("<http://x.test/u>, <mailto:u@x.test>", Some("List-Unsubscribe=One-Click")).unwrap();
        assert!(u.url.is_none() && !u.one_click);
        assert!(parse_unsubscribe("<http://x.test/u>", None).is_none());
        assert_eq!(parse_mailto("mailto:u%40x@x.test?subject=Please%20stop&body=bye+now"), Some(("u@x@x.test".into(), "Please stop".into(), "bye now".into())));
    }

    #[test]
    fn auth_results() {
        assert!(auth_failure("mx.x; spf=fail smtp.mailfrom=a.test; dkim=fail; dmarc=fail header.from=a.test").unwrap().contains("DMARC"));
        assert!(auth_failure("mx.x; spf=softfail smtp.mailfrom=a.test; dkim=none").is_some());
        assert!(auth_failure("mx.x; spf=softfail; dkim=pass").is_none());
        assert!(auth_failure("mx.x; spf=pass; dkim=pass; dmarc=pass").is_none());
    }

    #[test]
    fn links() {
        let html = r#"<p><a href="https://paypa1-secure.example/verify" style="x">https://www.paypal.com/verify</a>
            <a href="https://news.example.com/r?u=1">Read more</a>
            <a href='https://click.example.com/x'>example.com</a>
            <abbr>x</abbr><a href="https://evil.test/"><b>bank.com</b></a></p>"#;
        assert_eq!(deceptive_links(html), vec![("www.paypal.com".to_string(), "paypa1-secure.example".to_string()), ("bank.com".into(), "evil.test".into())]);
    }

    #[test]
    fn risky() {
        assert!(is_risky_attachment("Remittance.html", "text/html"));
        assert!(is_risky_attachment("invoice.pdf.exe", "application/octet-stream"));
        assert!(!is_risky_attachment("q3-budget.pdf", "application/pdf"));
    }
}
