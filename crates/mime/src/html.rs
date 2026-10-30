//! HTML: sanitizing for display (converting to text is in `html_text.rs`).

use super::*;

/// Most HTML we hand to the sanitizer or the text converter. Longer bodies are
/// cut (with a note) so one huge message can't stall sync or the reader.
pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// `s` cut to at most `max` bytes on a character boundary, and whether it was cut.
pub(crate) fn cap(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

pub struct Sanitized {
    pub html: String,
    /// The original referenced remote resources that were blocked.
    pub had_remote_content: bool,
}

/// Sanitize untrusted HTML. Scripts, event handlers, forms and frames are
/// removed. Remote images are dropped unless `allow_remote`; `cid:` images are
/// replaced from `inline` (cid -> data URL). The UI additionally renders the
/// result in a sandboxed iframe with a CSP, which also covers CSS `url()`.
pub fn sanitize_html(html: &str, allow_remote: bool, inline: &[(String, String)]) -> Sanitized {
    let had_remote = Arc::new(AtomicBool::new(false));
    let flag = had_remote.clone();
    let inline: Vec<(String, String)> = inline.to_vec();

    let mut b = ammonia::Builder::default();
    b.add_tags(["style", "font", "center", "big", "small", "u", "s"]);
    b.rm_clean_content_tags(["style"]);
    b.add_generic_attributes(["style", "class", "align", "valign", "width", "height", "bgcolor", "color", "dir", "role"]);
    b.add_tag_attributes("table", ["cellpadding", "cellspacing", "border"]);
    b.add_tag_attributes("td", ["colspan", "rowspan", "nowrap"]);
    b.add_tag_attributes("th", ["colspan", "rowspan"]);
    b.add_tag_attributes("font", ["face", "size"]);
    b.add_tag_attributes("img", ["src", "alt", "title", "width", "height"]);
    // Markers mail clients put on quoted history, so the reader can fold it.
    b.add_tag_attributes("div", ["id"]);
    b.add_tag_attributes("blockquote", ["type"]);
    b.url_schemes(HashSet::from(["http", "https", "mailto", "data", "cid", "tel"]));
    b.link_rel(Some("noopener noreferrer nofollow"));
    b.attribute_filter(move |element, attribute, value| {
        let lower = value.trim().to_ascii_lowercase();
        if element == "img" && attribute == "src" {
            if let Some(cid) = lower.strip_prefix("cid:") {
                let found = inline
                    .iter()
                    .find(|(c, _)| c.eq_ignore_ascii_case(cid))
                    .map(|(_, url)| url.clone());
                return found.map(Cow::Owned);
            }
            if lower.starts_with("data:image/") {
                return Some(Cow::Borrowed(value));
            }
            if lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("//") {
                flag.store(true, Ordering::Relaxed);
                return if allow_remote { Some(Cow::Borrowed(value)) } else { None };
            }
            return None;
        }
        if attribute == "style" && css_may_load(&lower) {
            flag.store(true, Ordering::Relaxed);
            if !allow_remote {
                // Escapes, image-set() and @import are rare in honest mail
                // and hard to strip reliably: drop the whole declaration list.
                if css_hard_to_strip(&lower) {
                    return None;
                }
                return Some(Cow::Owned(strip_css_urls(value)));
            }
        }
        if element == "a" && attribute == "href" && lower.starts_with("data:") {
            return None;
        }
        Some(Cow::Borrowed(value))
    });
    let (html, cut) = cap(html, MAX_HTML_BYTES);
    let mut out = b.clean(html).to_string();
    if !allow_remote {
        // <style> blocks may also pull remote resources.
        let (cleaned, found) = strip_risky_style_blocks(&out);
        if found {
            had_remote.store(true, Ordering::Relaxed);
            out = cleaned;
        }
        if out.to_ascii_lowercase().contains("url(") {
            had_remote.store(true, Ordering::Relaxed);
            out = strip_css_urls(&out);
        }
    }
    if cut {
        out.push_str("<p><em>[This message was too long to show in full.]</em></p>");
    }
    Sanitized { html: out, had_remote_content: had_remote.load(Ordering::Relaxed) }
}

/// CSS that may fetch something: `url()`, `image-set()`, `@import`, or
/// backslash escapes that could spell either (`\75rl(`). Takes lowercase input.
fn css_may_load(lower: &str) -> bool {
    lower.contains("url(") || css_hard_to_strip(lower)
}

/// The loading CSS that `strip_css_urls` can't rewrite. Takes lowercase input.
fn css_hard_to_strip(lower: &str) -> bool {
    lower.contains('\\') || lower.contains("image-set(") || lower.contains("@import")
}

/// Empty every `<style>` block in sanitized output whose CSS is hard to strip
/// (see `css_hard_to_strip`). Returns the new HTML and whether any was found.
pub(crate) fn strip_risky_style_blocks(s: &str) -> (String, bool) {
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut found = false;
    let mut pos = 0;
    while let Some(open) = lower[pos..].find("<style").map(|i| pos + i) {
        let Some(body) = lower[open..].find('>').map(|i| open + i + 1) else { break };
        let close = lower[body..].find("</style").map(|i| body + i).unwrap_or(s.len());
        out.push_str(&s[pos..body]);
        if css_hard_to_strip(&lower[body..close]) {
            found = true;
        } else {
            out.push_str(&s[body..close]);
        }
        pos = close;
    }
    out.push_str(&s[pos..]);
    (out, found)
}

pub(crate) fn strip_css_urls(s: &str) -> String {
    // ASCII lowercasing keeps byte offsets, so search `lower` and copy from `s`.
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("url(").map(|i| pos + i) {
        out.push_str(&s[pos..i]);
        match s[i + 4..].find(')') {
            Some(j) => {
                out.push_str("none");
                pos = i + 4 + j + 1;
            }
            None => {
                pos = s.len();
            }
        }
    }
    out.push_str(&s[pos..]);
    out
}
