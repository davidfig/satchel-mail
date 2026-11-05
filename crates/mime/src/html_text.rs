// SPDX-License-Identifier: MIT OR Apache-2.0
//! HTML → readable text for snippets, search and agents (0043): one pass
//! over html5ever's tokens. Blocks and table rows start new lines and cells
//! are spaced apart, so layout tables read top to bottom.

use std::cell::RefCell;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};

use super::*;

/// Readable text for HTML-only mail: paragraphs kept, links written inline as
/// `text (url)`, no markdown markers and no hard wrapping (the reader reflows).
pub fn html_to_text(html: &str) -> String {
    let (html, cut) = cap(html, MAX_HTML_BYTES);
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let tok = Tokenizer::new(Sink(RefCell::new(Text::default())), TokenizerOpts::default());
    let _ = tok.feed(&input);
    tok.end();
    let out = std::mem::take(&mut tok.sink.0.borrow_mut().out);
    let mut text = out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    if cut {
        text.push_str("\n\n[This message was too long to show in full.]");
    }
    text.trim().to_string()
}

/// Elements whose contents are never shown as text: skipped whole.
const SKIPPED: &[&str] = &["script", "style", "title", "template", "noembed", "noframes", "iframe", "svg", "math", "select", "textarea"];
/// Blocks set off by a blank line (lists too, unless nested).
const PARAGRAPHS: &[&str] = &["p", "h1", "h2", "h3", "h4", "h5", "h6", "table", "blockquote", "pre", "hr", "dl", "figure"];
/// Blocks that start on their own line.
const LINES: &[&str] = &[
    "div", "tr", "dt", "dd", "center", "section", "article", "header", "footer", "nav", "aside", "main", "form", "fieldset", "legend",
    "address", "figcaption", "details", "summary", "caption",
];

#[derive(Default)]
struct Text {
    out: String,
    /// Newlines owed before the next text: 1 for a new line, 2 or more for
    /// a blank one.
    newlines: usize,
    /// Collapsed whitespace owed before the next text.
    space: bool,
    /// The skipped element we're inside, if any.
    skip: Option<String>,
    pre: usize,
    quote: usize,
    /// The quote depth of the last text, so a blank line before a quote
    /// isn't marked as part of it.
    last_quote: usize,
    /// Open lists: the next item number, or None for bullets.
    lists: Vec<Option<i64>>,
    /// Open links: where their text starts in `out`, and the address to show.
    links: Vec<(usize, Option<String>)>,
}

impl Text {
    fn tag(&mut self, t: &Tag) -> TokenSinkResult<()> {
        let name = &*t.name;
        let start = t.kind == TagKind::StartTag;
        if let Some(skipped) = &self.skip {
            if !start && name == skipped {
                self.skip = None;
            }
            return TokenSinkResult::Continue;
        }
        if start && SKIPPED.contains(&name) {
            self.skip = Some(name.to_string());
            // Read these as raw text, as a browser does, so markup-looking
            // content can't end them early.
            return match name {
                "script" => TokenSinkResult::RawData(RawKind::ScriptData),
                "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
                "style" | "iframe" | "noembed" | "noframes" => TokenSinkResult::RawData(RawKind::Rawtext),
                _ => TokenSinkResult::Continue,
            };
        }
        match (name, start) {
            ("br", true) => self.line_break(),
            ("a", true) => {
                let href = attr(t, "href").unwrap_or_default();
                self.links.push((self.out.len(), shown_address(href.trim())));
            }
            ("a", false) => self.end_link(),
            ("pre", _) => self.pre = if start { self.pre + 1 } else { self.pre.saturating_sub(1) },
            ("blockquote", _) => self.quote = if start { self.quote + 1 } else { self.quote.saturating_sub(1) },
            ("ul" | "ol", true) => {
                let first = attr(t, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1);
                self.lists.push((name == "ol").then_some(first));
            }
            ("ul" | "ol", false) => {
                self.lists.pop();
            }
            ("li", true) => {
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "• ".into(),
                };
                self.newlines = self.newlines.max(1);
                self.write(&format!("{}{marker}", "  ".repeat(depth)));
            }
            ("li", false) => self.newlines = self.newlines.max(1),
            ("td" | "th", true) => self.space = true,
            _ => {}
        }
        let paragraph = match name {
            // Only the outermost list is set off; after a start tag it's
            // already on the stack, after an end tag it's gone.
            "ul" | "ol" => self.lists.len() <= usize::from(start),
            _ => PARAGRAPHS.contains(&name),
        };
        if paragraph {
            self.newlines = 2;
        } else if LINES.contains(&name) || matches!(name, "ul" | "ol") {
            self.newlines = self.newlines.max(1);
        }
        TokenSinkResult::Continue
    }

    fn chars(&mut self, s: &str) {
        if self.skip.is_some() {
            return;
        }
        if self.pre > 0 {
            for (i, line) in s.split('\n').enumerate() {
                if i > 0 {
                    self.line_break();
                }
                let line: String = line.chars().filter(|c| *c == '\t' || !hidden(*c)).collect();
                if !line.is_empty() {
                    self.write(&line);
                }
            }
            return;
        }
        let mut word = String::new();
        for c in s.chars() {
            if c.is_whitespace() {
                if !word.is_empty() {
                    self.write(&std::mem::take(&mut word));
                }
                self.space = true;
            } else if !hidden(c) {
                word.push(c);
            }
        }
        if !word.is_empty() {
            self.write(&word);
        }
    }

    /// A `<br>`: ends a line with text on it, or adds an empty line (so
    /// `<div><br></div>` is a blank line, as a browser shows it).
    fn line_break(&mut self) {
        self.newlines = if self.newlines == 0 { 1 } else { self.newlines + 1 };
    }

    /// Write text, first settling the breaks and space owed before it.
    fn write(&mut self, s: &str) {
        let breaks = self.newlines;
        if breaks > 0 && !self.out.is_empty() {
            self.out.push('\n');
            if breaks > 1 {
                self.out.push_str(&">".repeat(self.quote.min(self.last_quote)));
                self.out.push('\n');
            }
        }
        if self.out.is_empty() || self.out.ends_with('\n') {
            self.out.push_str(&"> ".repeat(self.quote));
        } else if self.space && !self.out.ends_with(' ') {
            self.out.push(' ');
        }
        self.newlines = 0;
        self.space = false;
        self.last_quote = self.quote;
        self.out.push_str(s);
    }

    /// Close a link: write its address after its text unless the text
    /// already says it. A link with no text (an image) adds nothing.
    fn end_link(&mut self) {
        let Some((start, Some(address))) = self.links.pop() else { return };
        let text = self.out.get(start..).unwrap_or_default().trim();
        if text.is_empty() || bare(text).eq_ignore_ascii_case(bare(&address)) {
            return;
        }
        self.out.push_str(&format!(" ({address})"));
    }
}

fn attr<'a>(t: &'a Tag, name: &str) -> Option<&'a str> {
    t.attrs.iter().find(|a| &*a.name.local == name).map(|a| &*a.value)
}

/// Invisible characters newsletters pad their previews with.
fn hidden(c: char) -> bool {
    is_hidden_char(c) || c == '\u{034F}'
}

/// What a link's `href` shows the reader: web and mail addresses only.
fn shown_address(href: &str) -> Option<String> {
    if href.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("mailto:")) {
        let addr = href[7..].split('?').next().unwrap_or_default();
        return (!addr.is_empty()).then(|| addr.to_string());
    }
    let lower = href.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")).then(|| href.to_string())
}

/// An address without its scheme and trailing slash, for comparing a link's
/// text with where it goes.
fn bare(s: &str) -> &str {
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).or_else(|| s.strip_prefix("mailto:")).unwrap_or(s);
    s.strip_suffix('/').unwrap_or(s)
}

struct Sink(RefCell<Text>);

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut t = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => return t.tag(&tag),
            Token::CharacterTokens(s) => t.chars(&s),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}
