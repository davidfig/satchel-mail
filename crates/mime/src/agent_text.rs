// SPDX-License-Identifier: MIT OR Apache-2.0
//! Message text for agents: the same words, without what costs tokens and
//! tells them nothing (0075). Links lose their tracking: Outlook safelinks
//! are unwrapped, tracking parameters dropped, and what's still long or goes
//! through a click tracker is cut to its host. Invisible padding goes, and
//! runs of blank lines become one.

/// A link longer than this, after its tracking is removed, is cut to its
/// host: what's left is an opaque token an agent can't use.
const MAX_LINK: usize = 100;

/// Query parameters that only identify the reader or the campaign. Generic
/// names (`id`, `cid`, `e`) are left alone: some sites need them.
const TRACKING_PARAMS: &[&str] = &[
    "fbclid", "gclid", "dclid", "msclkid", "igshid", "mc_cid", "mc_eid", "mkt_tok", "_hsenc", "_hsmi", "trackingid", "trackid", "lipi",
    "midtoken", "midsig", "refid", "trk", "trkemail", "otptoken", "ck_subscriber_id", "oly_enc_id", "oly_anon_id", "vero_id", "vero_conv",
    "s_cid", "cmpid", "sc_cid",
];

/// Prefixes of tracking parameters (`utm_source`, `pk_campaign`, …).
const TRACKING_PREFIXES: &[&str] = &["utm_", "pk_", "mtm_", "hsa_", "_bta_", "ga_"];

/// What a message added, as agents read it: its text without quoted
/// history, cleaned by [`agent_text`]. A plain-text part laid out in columns
/// (as some retailers send, with links chopped between them) gives way to
/// the HTML part's text.
pub fn agent_body(p: &crate::Parsed) -> String {
    agent_text(&crate::strip_quotes(&agent_source(&p.text, p.html.as_deref())))
}

/// The text agents should start from: the plain text, or the HTML part's
/// text when the plain text is laid out in columns.
pub fn agent_source<'a>(text: &'a str, html: Option<&str>) -> std::borrow::Cow<'a, str> {
    match html {
        Some(html) if laid_out(text) => crate::html_to_text(html).into(),
        _ => text.into(),
    }
}

/// Whether plain text is a page laid out with runs of spaces, not prose: a
/// fifth of its lines or more pad columns apart.
fn laid_out(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.len() >= 10 && lines.iter().filter(|l| l.trim().contains("        ")).count() * 5 >= lines.len()
}

/// `text` as agents read it. Quoted history isn't removed here; see
/// [`crate::strip_quotes`].
pub fn agent_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() / 2);
    let mut blank = true; // Also drops leading blank lines.
    for line in rejoin_links(text) {
        let line = squeeze_spaces(&clean_links(&line));
        let line = line.trim_end();
        if line.trim().is_empty() {
            if !blank {
                out.push('\n');
            }
            blank = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
        blank = false;
    }
    out.truncate(out.trim_end().len());
    out
}

/// Runs of three or more spaces inside a line (column padding) become two;
/// indentation stays.
fn squeeze_spaces(line: &str) -> String {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let mut out = String::from(&line[..indent]);
    let mut run = 0;
    for c in line[indent..].chars() {
        if c == ' ' {
            run += 1;
            if run <= 2 {
                out.push(c);
            }
        } else {
            run = 0;
            out.push(c);
        }
    }
    out
}

/// The lines without invisible padding, and with links that plain-text
/// parts hard-wrap over several lines put back on one.
fn rejoin_links(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    let mut open = false; // The last line ends inside a link.
    for line in text.lines() {
        let line: String = line
            .chars()
            .filter_map(|c| match c {
                '\t' | '\u{00A0}' | '\u{2007}' | '\u{202F}' => Some(' '),
                c if crate::is_hidden_char(c) || c == '\u{034F}' => None,
                c => Some(c),
            })
            .collect();
        if open && is_link_piece(&line) && let Some(last) = lines.last_mut() {
            last.push_str(line.trim());
            continue;
        }
        open = ends_in_link(&line);
        lines.push(line);
    }
    lines
}

/// Whether the line ends partway through a web address: one runs to its
/// last character.
fn ends_in_link(line: &str) -> bool {
    let line = line.trim_end();
    let mut rest = line;
    while let Some(i) = find_link(rest) {
        let tail = &rest[i..];
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        if end == tail.len() {
            return end > "https://".len();
        }
        rest = &tail[end..];
    }
    false
}

/// A line that can only be the next piece of a wrapped link: one token of
/// characters links use, that a word wouldn't look like.
fn is_link_piece(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 8
        && !t.contains(' ')
        && find_link(t).is_none()
        && t.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?#@!$&'()*+,;=%".contains(c))
        && t.chars().any(|c| c.is_ascii_digit() || "_=&%/".contains(c))
}

/// The line with each web address in it replaced by [`clean_link`]'s.
fn clean_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = find_link(rest) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = link_end(tail);
        out.push_str(&clean_link(&tail[..end]));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn find_link(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    [lower.find("https://"), lower.find("http://")].into_iter().flatten().min()
}

/// Where the link at the start of `s` ends: at whitespace or a character
/// that can't be in one, without trailing punctuation or a closing bracket
/// the link didn't open.
fn link_end(s: &str) -> usize {
    let mut end = s.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '[' | ']' | '{' | '}' | '|')).unwrap_or(s.len());
    loop {
        let link = &s[..end];
        let Some(last) = link.chars().last() else { break };
        let unbalanced = last == ')' && link.matches('(').count() < link.matches(')').count();
        if unbalanced || matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*') {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    end
}

/// A link as an agent needs it: where it really goes, without tracking, or
/// only its host when the rest is an opaque token.
pub fn clean_link(link: &str) -> String {
    let Some((scheme, rest)) = link.split_once("://") else { return link.to_string() };
    let (host_path, query) = rest.split_once('?').map_or((rest, None), |(a, b)| (a, Some(b)));
    let (host, path) = host_path.split_once('/').map_or((host_path, ""), |(h, p)| (h, p));
    let host_l = host.to_ascii_lowercase();
    // Outlook wraps every link in a safelink that holds the real one.
    if host_l.ends_with("safelinks.protection.outlook.com")
        && let Some(target) = query.and_then(|q| param(q, "url"))
        && find_link(&target) == Some(0)
    {
        return clean_link(&target);
    }
    if crate::trackers::is_click_tracker(&host_l, &format!("/{path}")) {
        return format!("{scheme}://{host}/…");
    }
    let kept: Vec<&str> = query.map(|q| q.split('&').filter(|kv| !kv.is_empty() && !is_tracking(kv.split('=').next().unwrap_or(kv))).collect()).unwrap_or_default();
    let mut clean = format!("{scheme}://{host_path}");
    if !kept.is_empty() {
        clean.push('?');
        clean.push_str(&kept.join("&"));
    }
    if clean.chars().count() > MAX_LINK {
        return format!("{scheme}://{host}/…");
    }
    clean
}

fn is_tracking(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    TRACKING_PARAMS.contains(&n.as_str()) || TRACKING_PREFIXES.iter().any(|p| n.starts_with(p))
}

/// A query parameter's decoded value.
fn param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| crate::safety::percent_decode(v, true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_lose_their_tracking() {
        assert_eq!(clean_link("https://example.com/a?utm_source=x&utm_medium=email&id=7"), "https://example.com/a?id=7");
        assert_eq!(clean_link("https://www.linkedin.com/comm/jobs/view/123?trackingId=abc&lipi=def&midToken=g"), "https://www.linkedin.com/comm/jobs/view/123");
        assert_eq!(clean_link("https://example.com/"), "https://example.com/");
        // Unwrapped, then cleaned in turn.
        assert_eq!(
            clean_link("https://na01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fshop.test%2Forder%2F42%3Futm_campaign%3Dz&data=05%7C01&reserved=0"),
            "https://shop.test/order/42"
        );
        // A safelink to something that isn't a web address stays a safelink.
        assert!(clean_link("https://na01.safelinks.protection.outlook.com/?url=javascript%3Aalert(1)&data=x").contains("safelinks"));
        assert_eq!(clean_link("https://u1.ct.sendgrid.net/ls/click?upn=abc"), "https://u1.ct.sendgrid.net/…");
        let opaque = format!("https://links.e.shop.test/els/{}", "x".repeat(200));
        assert_eq!(clean_link(&opaque), "https://links.e.shop.test/…");
    }

    #[test]
    fn column_layouts_give_way_to_the_html() {
        let row = "Deal                      (https://shop.test/click?params=v1.ab\n";
        let mut p = crate::Parsed { text: row.repeat(12), html: Some("<p>Deal of the week</p>".into()), ..Default::default() };
        assert_eq!(agent_body(&p), "Deal of the week");
        p.text = "Hi Bob,\n\nSee you Monday.\n\nOn Mon, Bob wrote:\n> Lunch?".into();
        assert_eq!(agent_body(&p), "Hi Bob,\n\nSee you Monday.", "prose keeps its own text, without the quote");
    }

    #[test]
    fn text_keeps_its_words() {
        let text = "\u{200B}\u{034F}Hello,\u{00A0}Bob\n\n\n\nSee https://example.com/report?utm_source=x.\n   \n(details at https://en.wikipedia.org/wiki/Rust_(language))\n\n";
        assert_eq!(agent_text(text), "Hello, Bob\n\nSee https://example.com/report.\n\n(details at https://en.wikipedia.org/wiki/Rust_(language))");
        assert_eq!(agent_text("Markdown [shop](https://shop.test/x?fbclid=1) link"), "Markdown [shop](https://shop.test/x) link");
        assert_eq!(agent_text("HTTPS://SHOP.TEST/A?utm_x=1"), "HTTPS://SHOP.TEST/A");
        assert_eq!(agent_text("no links here"), "no links here");
        assert_eq!(agent_text("    Item          $4.99"), "    Item  $4.99");
        // A plain-text part that wraps a long link over several lines.
        let wrapped = "Unsubscribe (https://data.shop.test/click?params=v1.abc_DEF\n6lOlpEttto_ppKiW-3L-g\n&p=v1.xvVJpIlQxYox0ql\n394aTPzaGEkbYUpw)\nThanks for shopping\n2026";
        assert_eq!(agent_text(wrapped), "Unsubscribe (https://data.shop.test/…)\nThanks for shopping\n2026");
        // Words after a link that ends its line stay where they are.
        assert_eq!(agent_text("See https://shop.test/a\nThanks,\nBob"), "See https://shop.test/a\nThanks,\nBob");
    }
}
