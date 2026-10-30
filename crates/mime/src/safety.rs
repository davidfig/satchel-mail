//! Signals about whether a message is safe: unsubscribe headers, sender
//! authentication results, deceptive links and risky attachments. These only
//! read the message; deciding what to show is the core's job.

use satchel_api::{Unsubscribe, UnsubscribeEmail};

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

/// File types that run code, open something else (shortcuts, disk images),
/// or imitate a web page when opened.
const RISKY_EXT: &[&str] = &[
    // Web pages and scripts.
    "html", "htm", "shtml", "xhtml", "mht", "mhtml", "svg", "hta", "js", "jse", "vbs", "vbe", "wsf", "wsh", "ps1", "psm1", "jar",
    // Windows programs, installers, shortcuts and settings.
    "exe", "scr", "com", "bat", "cmd", "pif", "msi", "msp", "msix", "msixbundle", "appx", "appxbundle", "dll", "cpl", "msc", "reg", "chm",
    "lnk", "url", "inf", "gadget", "application", "settingcontent-ms", "library-ms", "search-ms",
    // Office files with macros or add-ins.
    "docm", "dotm", "xlsm", "xltm", "xlam", "xll", "pptm", "potm", "ppam", "one",
    // macOS programs, scripts, installers and shortcuts.
    "app", "pkg", "mpkg", "command", "tool", "sh", "scpt", "scptd", "applescript", "workflow", "terminal", "webloc", "inetloc", "fileloc",
    // Disk images, which mount and can hide any of the above.
    "iso", "img", "vhd", "vhdx", "dmg",
    // Profiles, remote sessions, app installers and launchers, web queries.
    "mobileconfig", "rdp", "appinstaller", "appref-ms", "jnlp", "iqy", "slk", "xht", "svgz", "wsc", "sct",
];

/// Archives, which can hold any of the risky types out of sight.
const ARCHIVE_EXT: &[&str] = &["zip", "zipx", "7z", "rar", "cab", "ace", "arj", "lzh", "tar", "gz", "tgz", "bz2", "xz"];

/// The extension a file really opens as: Windows and macOS ignore trailing
/// dots and spaces ("invoice.exe." runs as an .exe), and an NTFS stream
/// suffix ("x.exe::$DATA") doesn't change the type.
fn real_extension(filename: &str) -> String {
    let name = clean_filename(filename).to_ascii_lowercase();
    let name = name.trim_end_matches(['.', ' ']);
    let name = name.split(':').next().unwrap_or(name);
    name.rsplit_once('.').map(|(_, e)| e.trim().to_string()).unwrap_or_default()
}

/// Attachments that can run code or imitate a web page when opened, or are
/// archives that can hide such files.
pub fn is_risky_attachment(filename: &str, mime_type: &str) -> bool {
    let ext = real_extension(filename);
    if RISKY_EXT.contains(&ext.as_str()) || ARCHIVE_EXT.contains(&ext.as_str()) {
        return true;
    }
    let m = mime_type.to_ascii_lowercase();
    matches!(
        m.as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "image/svg+xml"
            | "application/hta"
            | "application/x-msdownload"
            | "application/x-msdos-program"
            | "application/x-ms-installer"
            | "application/x-msi"
            | "application/vnd.microsoft.portable-executable"
            | "application/javascript"
            | "text/javascript"
            | "application/x-sh"
            | "application/x-apple-diskimage"
            | "application/x-iso9660-image"
            | "application/zip"
            | "application/x-zip-compressed"
            | "application/x-7z-compressed"
            | "application/vnd.rar"
            | "application/x-rar-compressed"
            | "application/gzip"
            | "application/x-gzip"
            | "application/x-tar"
            | "application/x-bzip2"
            | "application/x-xz"
            | "application/x-apple-aspen-config"
            | "application/x-java-jnlp-file"
    )
}

/// Whether a risky attachment is an archive (it may hold risky files rather
/// than be one).
pub fn is_archive(filename: &str) -> bool {
    ARCHIVE_EXT.contains(&real_extension(filename).as_str())
}

/// A filename safe to show and to save: without control characters or the
/// invisible direction and formatting marks that make "invoice\u{202E}fdp.exe"
/// display as "invoiceexe.pdf".
pub fn clean_filename(name: &str) -> String {
    name.chars().filter(|&c| !is_hidden_char(c)).collect::<String>().trim().to_string()
}

/// Control characters (C0, DEL, C1) and Unicode format characters that
/// reorder or hide text: bidi embeddings, overrides, isolates and marks,
/// zero-width characters and the BOM.
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{061C}' | '\u{FEFF}' | '\u{00AD}')
}

/// Parse List-Unsubscribe (and List-Unsubscribe-Post for one-click). A mailto
/// is kept only if it names exactly one valid address.
pub fn parse_unsubscribe(list_unsubscribe: &str, post: Option<&str>) -> Option<Unsubscribe> {
    let mut u = Unsubscribe::default();
    for part in list_unsubscribe.split(',') {
        let v = part.trim().trim_start_matches('<').trim_end_matches('>').trim();
        let lower = v.to_ascii_lowercase();
        if lower.starts_with("https://") && u.url.is_none() {
            u.url = Some(v.to_string());
        } else if lower.starts_with("mailto:")
            && u.mailto.is_none()
            && let Some((to, subject, body)) = parse_mailto(v)
        {
            u.mailto = Some(v.to_string());
            let or_default = |s: String| if s.is_empty() { "unsubscribe".to_string() } else { s };
            u.email = Some(UnsubscribeEmail { to, subject: or_default(subject), body: or_default(body), other_domain: false });
        }
    }
    // RFC 8058: one-click needs the exact POST body and an https URL.
    u.one_click = u.url.is_some() && post.is_some_and(|p| p.split_whitespace().collect::<String>().eq_ignore_ascii_case("List-Unsubscribe=One-Click"));
    (u.url.is_some() || u.mailto.is_some()).then_some(u)
}

/// Longest subject and body taken from a mailto: link.
pub(crate) const MAILTO_SUBJECT_MAX: usize = 200;
pub(crate) const MAILTO_BODY_MAX: usize = 1000;

/// A mailto: URL as (address, subject, body). The address must be exactly
/// one plain addr-spec: anything that could add recipients or headers
/// ("a@x>, <b@y", quotes, separators, control characters) is refused. Line
/// breaks and other control characters in subject and body become spaces,
/// and both are capped in length.
pub fn parse_mailto(url: &str) -> Option<(String, String, String)> {
    let rest = url.get(..7).filter(|s| s.eq_ignore_ascii_case("mailto:")).map(|_| &url[7..])?;
    let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
    let addr = percent_decode(addr, false);
    if !is_plain_address(&addr) {
        return None;
    }
    let (mut subject, mut body) = (String::new(), String::new());
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k.to_ascii_lowercase().as_str() {
            "subject" => subject = one_line(&percent_decode(v, true), MAILTO_SUBJECT_MAX),
            "body" => body = one_line(&percent_decode(v, true), MAILTO_BODY_MAX),
            _ => {}
        }
    }
    Some((addr, subject, body))
}

/// One local@domain address with nothing that could start another address,
/// a display name, a comment or a header.
fn is_plain_address(addr: &str) -> bool {
    let Some((local, host)) = addr.split_once('@') else { return false };
    let bad = |c: char| c.is_whitespace() || is_hidden_char(c) || "<>,;:\"\\()[]@".contains(c);
    !local.is_empty()
        && !host.is_empty()
        && addr.len() <= 254
        && !local.chars().any(bad)
        && host.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if is_hidden_char(c) { ' ' } else { c }).collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Decodes %XX escapes; `plus_is_space` for form-style query values.
fn percent_decode(s: &str, plus_is_space: bool) -> String {
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
        out.push(if plus_is_space && b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
        let href = attr(&html[start..tag_end], "href").map(|h| decode_entities(&h));
        let text = decode_entities(&strip_tags(&html[tag_end + 1..close]));
        pos = close;
        let (Some(href), Some(shown)) = (href, shown_host(&text)) else { continue };
        let Some(actual) = url_host(&unwrap_safe_link(&href)) else { continue };
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

/// Decode HTML character references the way a browser shows them: numeric
/// (`&#46;`, `&#x2E;`, with or without the `;`) and the named ones that can
/// spell a domain or hide in one. Then drop invisible characters, and map
/// the dots other scripts use (`。`, `．`) to `.`, so the text compares as it reads.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match decode_one(rest) {
            Some((c, used)) => {
                out.extend(c);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out.chars()
        .filter(|&c| !is_invisible(c))
        .map(|c| if matches!(c, '\u{3002}' | '\u{FF0E}' | '\u{FF61}') { '.' } else { c })
        .collect()
}

/// One character reference at the start of `s` (which starts with `&`):
/// the character (None for one that decodes to nothing) and bytes used.
fn decode_one(s: &str) -> Option<(Option<char>, usize)> {
    let body = &s[1..];
    if let Some(num) = body.strip_prefix('#') {
        let (hex, digits) = match num.strip_prefix(['x', 'X']) {
            Some(h) => (true, h),
            None => (false, num),
        };
        let len = digits.find(|c: char| !(if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() })).unwrap_or(digits.len());
        if len == 0 {
            return None;
        }
        let v = u32::from_str_radix(&digits[..len.min(8)], if hex { 16 } else { 10 }).unwrap_or(0xFFFD);
        let c = char::from_u32(v).filter(|&c| c != '\0').unwrap_or('\u{FFFD}');
        let used = 1 + 1 + usize::from(hex) + len + usize::from(digits[len..].starts_with(';'));
        return Some((Some(c), used));
    }
    let len = body.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(body.len());
    if !body[len..].starts_with(';') {
        // A few legacy names also work without the `;`.
        let name = &body[..len];
        let c = match name {
            "amp" | "AMP" => '&',
            "lt" | "LT" => '<',
            "gt" | "GT" => '>',
            "quot" | "QUOT" => '"',
            "nbsp" => ' ',
            "shy" => return Some((None, 1 + len)),
            _ => return None,
        };
        return Some((Some(c), 1 + len));
    }
    let c = match &body[..len] {
        "amp" | "AMP" => '&',
        "lt" | "LT" => '<',
        "gt" | "GT" => '>',
        "quot" | "QUOT" => '"',
        "apos" => '\'',
        "nbsp" | "NonBreakingSpace" => ' ',
        "period" => '.',
        "sol" => '/',
        "bsol" => '\\',
        "colon" => ':',
        "commat" => '@',
        "num" => '#',
        "quest" => '?',
        "equals" => '=',
        "percnt" => '%',
        "lowbar" | "UnderBar" => '_',
        "hyphen" | "dash" => '\u{2010}',
        "minus" => '\u{2212}',
        "middot" | "centerdot" | "CenterDot" => '\u{B7}',
        "shy" | "zwj" | "zwnj" | "lrm" | "rlm" | "ZeroWidthSpace" | "NegativeVeryThinSpace" | "NegativeThinSpace" | "NegativeMediumSpace" | "NegativeThickSpace" | "NoBreak" | "it" | "InvisibleTimes" | "ic" | "InvisibleComma" | "af" | "ApplyFunction" => {
            return Some((None, 2 + len));
        }
        _ => return None,
    };
    Some((Some(c), 2 + len))
}

/// Characters that show as nothing: the hidden ones `clean_filename` drops,
/// plus fillers and joiners that render blank.
fn is_invisible(c: char) -> bool {
    is_hidden_char(c) || matches!(c, '\u{034F}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180B}'..='\u{180E}' | '\u{3164}' | '\u{FFA0}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0000}'..='\u{E007F}')
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

/// The real destination behind Microsoft Safe Links
/// (`*.safelinks.protection.outlook.com/?url=…`), which Outlook and Hotmail
/// wrap every link in. Other hrefs come back unchanged.
fn unwrap_safe_link(href: &str) -> String {
    let mut href = href.trim().to_string();
    // A forwarded message can wrap a link more than once.
    for _ in 0..3 {
        let Some(host) = url_host(&href) else { break };
        if !host.ends_with(".safelinks.protection.outlook.com") {
            break;
        }
        let query = href.split_once('?').map(|(_, q)| q.split('#').next().unwrap_or("")).unwrap_or("");
        let Some(inner) = query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case("url")).map(|(_, v)| percent_decode(v, false))) else { break };
        href = inner;
    }
    href
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
        assert_eq!(parse_mailto("mailto:u@x.test?subject=Please%20stop&body=bye+now"), Some(("u@x.test".into(), "Please stop".into(), "bye now".into())));
    }

    #[test]
    fn links() {
        let html = r#"<p><a href="https://paypa1-secure.example/verify" style="x">https://www.paypal.com/verify</a>
            <a href="https://news.example.com/r?u=1">Read more</a>
            <a href='https://click.example.com/x'>example.com</a>
            <abbr>x</abbr><a href="https://evil.test/"><b>bank.com</b></a></p>"#;
        assert_eq!(deceptive_links(html), vec![("www.paypal.com".to_string(), "paypa1-secure.example".to_string()), ("bank.com".into(), "evil.test".into())]);
        // Entities and invisible characters don't hide the shown domain.
        for text in ["bank&#x2E;com", "bank&#46com", "bank&period;com", "ba&ZeroWidthSpace;nk.com", "bank\u{200B}.c\u{2060}om", "bank&shy;.com", "bank\u{3002}com", "&#98;ank.com"] {
            let html = format!(r#"<a href="https://evil.test/">{text}</a>"#);
            assert_eq!(deceptive_links(&html), vec![("bank.com".to_string(), "evil.test".to_string())], "{text}");
        }
        // An encoded href is decoded too; a double-encoded entity stays literal.
        assert_eq!(deceptive_links(r#"<a href="https&#58;//evil.test/">bank.com</a>"#).len(), 1);
        assert!(deceptive_links(r#"<a href="https://evil.test/">bank&amp;#46;com</a>"#).is_empty());
        // Safe Links wrappers are judged by where they really go.
        let wrap = |u: &str| format!("https://nam12.safelinks.protection.outlook.com/?url={u}&amp;data=05%7C02&amp;reserved=0");
        assert!(deceptive_links(&format!(r#"<a href="{}">wsdot.com</a>"#, wrap("https%3A%2F%2Fwww.wsdot.com%2Ftraffic"))).is_empty());
        let nested = wrap(&wrap("https%3A%2F%2Fwsdot.com%2F").replace('%', "%25").replace('&', "%26").replace('?', "%3F").replace('=', "%3D"));
        assert!(deceptive_links(&format!(r#"<a href="{nested}">wsdot.com</a>"#)).is_empty());
        assert_eq!(deceptive_links(&format!(r#"<a href="{}">wsdot.com</a>"#, wrap("https%3A%2F%2Fevil.test%2F"))), vec![("wsdot.com".to_string(), "evil.test".to_string())]);
        assert_eq!(deceptive_links(r#"<a href="https://safelinks.protection.outlook.com.evil.test/?url=https%3A%2F%2Fwsdot.com">wsdot.com</a>"#).len(), 1);
        assert_eq!(decode_entities("&#x1F600;&#0;&#99999999999;&unknown;&amp"), "\u{1F600}\u{FFFD}\u{FFFD}&unknown;&");
    }

    #[test]
    fn risky() {
        assert!(is_risky_attachment("Remittance.html", "text/html"));
        assert!(is_risky_attachment("invoice.pdf.exe", "application/octet-stream"));
        assert!(!is_risky_attachment("q3-budget.pdf", "application/pdf"));
        // Windows and macOS ignore trailing dots and spaces.
        assert!(is_risky_attachment("invoice.exe.", "application/octet-stream"));
        assert!(is_risky_attachment("invoice.exe . .", "application/octet-stream"));
        assert!(is_risky_attachment("invoice.exe::$DATA", "application/octet-stream"));
        for name in ["run.scpt", "x.terminal", "go.webloc", "a.inetloc", "b.fileloc", "c.workflow", "d.settingcontent-ms", "e.library-ms", "f.vhdx", "g.appxbundle", "h.xll", "i.reg", "j.url", "k.mpkg", "l.applescript", "m.cpl"] {
            assert!(is_risky_attachment(name, "application/octet-stream"), "{name}");
        }
        // Archives can hide any of these.
        assert!(is_risky_attachment("photos.zip", "application/octet-stream") && is_archive("photos.ZIP"));
        assert!(!is_archive("setup.exe"));
        for name in ["a.mobileconfig", "b.rdp", "c.appinstaller", "d.appref-ms", "e.jnlp", "f.iqy", "g.slk", "h.xht", "i.svgz", "j.wsc", "k.sct", "l.tar", "m.gz", "n.tgz", "o.bz2", "p.xz", "q.zipx", "r.lzh"] {
            assert!(is_risky_attachment(name, "application/octet-stream"), "{name}");
        }
        assert!(is_archive("backup.tar.gz"));
        // A right-to-left override makes "invoice\u{202E}fdp.exe" show as "invoiceexe.pdf".
        let spoof = "invoice\u{202E}fdp.exe";
        assert!(is_risky_attachment(spoof, "application/pdf"));
        assert_eq!(clean_filename(spoof), "invoicefdp.exe");
        assert_eq!(clean_filename("a\u{2066}b\u{200F}c\u{0007}\u{0085}.pdf"), "abc.pdf");
    }
}
