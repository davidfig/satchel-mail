//! HTML: sanitizing for display, and converting to text.

use super::*;

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
        if attribute == "style" && lower.contains("url(") {
            flag.store(true, Ordering::Relaxed);
            if !allow_remote {
                return Some(Cow::Owned(strip_css_urls(value)));
            }
        }
        if element == "a" && attribute == "href" && lower.starts_with("data:") {
            return None;
        }
        Some(Cow::Borrowed(value))
    });
    let mut out = b.clean(html).to_string();
    if !allow_remote && out.contains("url(") {
        // <style> blocks may also pull remote resources.
        had_remote.store(true, Ordering::Relaxed);
        out = strip_css_urls(&out);
    }
    Sanitized { html: out, had_remote_content: had_remote.load(Ordering::Relaxed) }
}

pub(crate) fn strip_css_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.to_ascii_lowercase().find("url(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 4..];
        match after.find(')') {
            Some(j) => {
                out.push_str("none");
                rest = &after[j + 1..];
            }
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Readable text for HTML-only mail: paragraphs kept, links written inline as
/// `text (url)`, no markdown markers and no hard wrapping (the reader reflows).
pub fn html_to_text(html: &str) -> String {
    let urls = Rc::new(RefCell::new(Vec::new()));
    let text = html2text::config::with_decorator(MailTextDecorator { links: vec![], urls: urls.clone() })
        .no_table_borders()
        .no_link_wrapping()
        .allow_width_overflow()
        .string_from_read(html.as_bytes(), 10_000)
        .unwrap_or_default();
    let mut text = text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    // A link whose text is its own address doesn't need it twice.
    for url in urls.borrow().iter() {
        text = text.replace(&format!("{url} ({url})"), url);
    }
    text.trim().to_string()
}

#[derive(Clone)]
pub(crate) struct MailTextDecorator {
    /// What to write at the end of each open link.
    links: Vec<String>,
    urls: Rc<RefCell<Vec<String>>>,
}

impl html2text::render::TextDecorator for MailTextDecorator {
    type Annotation = ();

    fn decorate_link_start(&mut self, url: &str) -> (String, ()) {
        let target = url.strip_prefix("mailto:").unwrap_or(url);
        let shown = url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:");
        self.links.push(if shown { format!(" ({target})") } else { String::new() });
        if shown {
            self.urls.borrow_mut().push(target.to_string());
        }
        (String::new(), ())
    }
    fn decorate_link_end(&mut self) -> String {
        self.links.pop().unwrap_or_default()
    }
    fn decorate_em_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_em_end(&self) -> String {
        String::new()
    }
    fn decorate_strong_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_strong_end(&self) -> String {
        String::new()
    }
    fn decorate_strikeout_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_strikeout_end(&self) -> String {
        String::new()
    }
    fn decorate_code_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_code_end(&self) -> String {
        String::new()
    }
    fn decorate_preformat_first(&self) {}
    fn decorate_preformat_cont(&self) {}
    // Images are left out so an image-only message still has no text.
    fn decorate_image(&mut self, _src: &str, _title: &str) -> (String, ()) {
        (String::new(), ())
    }
    fn header_prefix(&self, _level: usize) -> String {
        String::new()
    }
    fn quote_prefix(&self) -> String {
        "> ".into()
    }
    fn unordered_item_prefix(&self) -> String {
        "• ".into()
    }
    fn ordered_item_prefix(&self, i: i64) -> String {
        format!("{i}. ")
    }
    fn make_subblock_decorator(&self) -> Self {
        Self { links: vec![], urls: self.urls.clone() }
    }
}
