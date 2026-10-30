//! vCard (RFC 6350, and the 3.0 dialect most servers still speak) for the
//! properties Satchel edits: FN/N, EMAIL, TEL, ORG, TITLE, BDAY, NOTE.
//!
//! Cards from a server are patched, not rewritten: every property we don't
//! edit (photos, postal addresses, URLs, X- extensions) is kept as written,
//! and properties whose value didn't change keep their original line.

use satchel_api::{ContactEmail, ContactFields, ContactPhone};

use super::{escape, fold, split_unquoted, unescape, unfold};

/// The parts of a card Satchel understands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedCard {
    pub uid: Option<String>,
    pub version: Option<String>,
    pub fields: ContactFields,
}

/// One content line: `group.NAME;params:value`.
#[derive(Debug, Clone)]
struct Line {
    group: Option<String>,
    name: String,
    /// (NAME, value) pairs; bare 3.0 params (`EMAIL;WORK:`) become ("TYPE", value).
    params: Vec<(String, String)>,
    value: String,
    raw: String,
}

impl Line {
    fn types(&self) -> Vec<String> {
        self.params
            .iter()
            .filter(|(k, _)| k == "TYPE")
            .flat_map(|(_, v)| v.split(','))
            .map(|t| t.trim().trim_matches('"').to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }
    fn param(&self, k: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

fn parse_line(raw: &str) -> Option<Line> {
    let mut in_quote = false;
    let colon = raw.char_indices().find_map(|(i, c)| match c {
        '"' => {
            in_quote = !in_quote;
            None
        }
        ':' if !in_quote => Some(i),
        _ => None,
    })?;
    let (head, value) = (&raw[..colon], &raw[colon + 1..]);
    let mut parts = split_unquoted(head, ';');
    let full = parts.remove(0).to_ascii_uppercase();
    let (group, name) = match full.rsplit_once('.') {
        Some((g, n)) => (Some(g.to_string()), n.to_string()),
        None => (None, full),
    };
    let params = parts
        .into_iter()
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_uppercase(), v.trim_matches('"').to_string()),
            None => ("TYPE".to_string(), p.trim().to_string()),
        })
        .collect();
    Some(Line { group, name, params, value: value.to_string(), raw: raw.to_string() })
}

/// The lines of the first `BEGIN:VCARD` … `END:VCARD` block, without the fences.
fn card_lines(text: &str) -> Option<Vec<Line>> {
    let mut out = vec![];
    let mut inside = false;
    for raw in unfold(text) {
        let Some(l) = parse_line(&raw) else { continue };
        match (l.name.as_str(), l.value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VCARD") if !inside => inside = true,
            ("END", "VCARD") if inside => return Some(out),
            _ if inside => out.push(l),
            _ => {}
        }
    }
    inside.then_some(out)
}

/// `_$!<Mobile>!$_` (Apple's built-in labels) or a custom label, lowercased.
fn ab_label(v: &str) -> String {
    let v = unescape(v);
    let inner = v.strip_prefix("_$!<").and_then(|s| s.strip_suffix(">!$_")).unwrap_or(&v);
    inner.trim().to_lowercase()
}

const NOISE_TYPES: [&str; 6] = ["internet", "pref", "voice", "x400", "text", "uri"];

fn label_of(l: &Line, ab_labels: &[(String, String)]) -> Option<String> {
    if let Some(g) = &l.group
        && let Some((_, label)) = ab_labels.iter().find(|(gr, _)| gr == g)
    {
        return Some(label.clone());
    }
    let types = l.types();
    if types.iter().any(|t| t == "cell" || t == "mobile" || t == "iphone") {
        return Some("cell".into());
    }
    types.into_iter().find(|t| !NOISE_TYPES.contains(&t.as_str()))
}

/// First component of a structured value (`ORG:Acme;Sales` → `Acme`).
fn first_component(v: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for c in v.chars() {
        if esc {
            out.push('\\');
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ';' {
            break;
        } else {
            out.push(c);
        }
    }
    unescape(&out).trim().to_string()
}

/// `1980-04-02`, `19800402`, `--0402`, `--04-02` or a date-time; `--MM-DD`
/// for a birthday without a year. Apple's omitted year (1604) counts as none.
pub fn normalize_birthday(v: &str, omit_year: Option<&str>) -> Option<String> {
    let v = v.trim();
    let date = v.split('T').next().unwrap_or(v);
    if let Some(rest) = date.strip_prefix("--") {
        let d: String = rest.chars().filter(char::is_ascii_digit).collect();
        return (d.len() == 4).then(|| format!("--{}-{}", &d[..2], &d[2..]));
    }
    let d: String = date.chars().filter(char::is_ascii_digit).collect();
    if d.len() != 8 {
        return None;
    }
    let (y, m, day) = (&d[..4], &d[4..6], &d[6..]);
    if omit_year == Some(y) {
        return Some(format!("--{m}-{day}"));
    }
    Some(format!("{y}-{m}-{day}"))
}

/// Parse the first card in `text`.
pub fn parse(text: &str) -> Option<ParsedCard> {
    let lines = card_lines(text)?;
    let ab_labels: Vec<(String, String)> =
        lines.iter().filter(|l| l.name == "X-ABLABEL").filter_map(|l| Some((l.group.clone()?, ab_label(&l.value)))).collect();
    let mut card = ParsedCard::default();
    let mut n_name = None;
    for l in &lines {
        match l.name.as_str() {
            "UID" => card.uid = Some(unescape(&l.value)).filter(|s| !s.is_empty()),
            "VERSION" => card.version = Some(l.value.trim().to_string()),
            "FN" => card.fields.name = unescape(&l.value).trim().to_string(),
            "N" => {
                let parts: Vec<String> = split_unquoted(&l.value, ';').iter().map(|p| unescape(p).trim().to_string()).collect();
                let (family, given) = (parts.first().cloned().unwrap_or_default(), parts.get(1).cloned().unwrap_or_default());
                let joined = [given, family].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                n_name = Some(joined).filter(|s| !s.is_empty());
            }
            "EMAIL" => {
                let email = unescape(&l.value).trim().trim_start_matches("mailto:").to_string();
                if !email.is_empty() {
                    card.fields.emails.push(ContactEmail { email, label: label_of(l, &ab_labels) });
                }
            }
            "TEL" => {
                let number = unescape(&l.value).trim().trim_start_matches("tel:").to_string();
                if !number.is_empty() {
                    card.fields.phones.push(ContactPhone { number, label: label_of(l, &ab_labels) });
                }
            }
            "ORG" => card.fields.organization = first_component(&l.value),
            "TITLE" => card.fields.title = unescape(&l.value).trim().to_string(),
            "NOTE" => card.fields.notes = unescape(&l.value).trim().to_string(),
            "BDAY" => card.fields.birthday = normalize_birthday(&l.value, l.param("X-APPLE-OMIT-YEAR")),
            _ => {}
        }
    }
    if card.fields.name.is_empty()
        && let Some(n) = n_name
    {
        card.fields.name = n;
    }
    Some(card)
}

/// `N` from a display name: the last word is the family name.
fn n_value(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let (given, family) = match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => (rest.join(" "), last.to_string()),
        Some((only, _)) => (only.to_string(), String::new()),
        None => (String::new(), String::new()),
    };
    format!("{};{};;;", escape(&family), escape(&given))
}

fn typed(name: &str, label: &Option<String>, value: &str) -> String {
    match label.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) if l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => format!("{name};TYPE={}:{value}", l.to_lowercase()),
        _ => format!("{name}:{value}"),
    }
}

/// The lines for the properties in `f` that `keep` says are unchanged
/// are taken from the original; the rest are generated.
fn generated(f: &ContactFields, old: Option<&ContactFields>) -> (Vec<&'static str>, Vec<String>) {
    let same = |pick: &dyn Fn(&ContactFields) -> bool| old.is_some_and(pick);
    let mut replace: Vec<&'static str> = vec![];
    let mut out: Vec<String> = vec![];
    if !same(&|o| o.name == f.name) {
        replace.extend(["FN", "N"]);
        out.push(format!("FN:{}", escape(&f.name)));
        out.push(format!("N:{}", n_value(&f.name)));
    }
    if !same(&|o| o.emails == f.emails) {
        replace.push("EMAIL");
        out.extend(f.emails.iter().filter(|e| !e.email.trim().is_empty()).map(|e| typed("EMAIL", &e.label, e.email.trim())));
    }
    if !same(&|o| o.phones == f.phones) {
        replace.push("TEL");
        out.extend(f.phones.iter().filter(|p| !p.number.trim().is_empty()).map(|p| typed("TEL", &p.label, &escape(p.number.trim()))));
    }
    if !same(&|o| o.organization == f.organization) {
        replace.push("ORG");
        if !f.organization.trim().is_empty() {
            out.push(format!("ORG:{}", escape(f.organization.trim())));
        }
    }
    if !same(&|o| o.title == f.title) {
        replace.push("TITLE");
        if !f.title.trim().is_empty() {
            out.push(format!("TITLE:{}", escape(f.title.trim())));
        }
    }
    if !same(&|o| o.birthday == f.birthday) {
        replace.push("BDAY");
        if let Some(b) = f.birthday.as_deref().and_then(|b| normalize_birthday(b, None)) {
            out.push(format!("BDAY:{b}"));
        }
    }
    if !same(&|o| o.notes == f.notes) {
        replace.push("NOTE");
        if !f.notes.trim().is_empty() {
            out.push(format!("NOTE:{}", escape(f.notes.trim())));
        }
    }
    (replace, out)
}

/// Build a card for `fields`. With `original`, that card is patched: its
/// other properties (and unchanged ones) are kept as written.
pub fn build(original: Option<&str>, uid: &str, fields: &ContactFields) -> String {
    let old_lines = original.and_then(card_lines);
    let old = original.and_then(parse);
    let (replace, new) = generated(fields, old.as_ref().map(|c| &c.fields));
    let mut lines: Vec<String> = vec!["BEGIN:VCARD".into()];
    match old_lines {
        Some(old_lines) => {
            // Apple puts labels in X-ABLabel lines grouped with their property.
            let dropped_groups: Vec<String> =
                old_lines.iter().filter(|l| replace.contains(&l.name.as_str())).filter_map(|l| l.group.clone()).collect();
            let mut has_uid = false;
            for l in &old_lines {
                if replace.contains(&l.name.as_str()) || (l.name == "X-ABLABEL" && l.group.as_ref().is_some_and(|g| dropped_groups.contains(g))) {
                    continue;
                }
                has_uid |= l.name == "UID";
                lines.push(l.raw.clone());
            }
            if !has_uid {
                lines.push(format!("UID:{}", escape(uid)));
            }
        }
        None => {
            lines.push("VERSION:3.0".into());
            lines.push("PRODID:-//satchel.email//EN".into());
            lines.push(format!("UID:{}", escape(uid)));
        }
    }
    lines.extend(new);
    lines.push("END:VCARD".into());
    lines.iter().map(|l| fold(l)).collect()
}

pub fn new_uid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Apple Inc.//iOS 17//EN\r\nN:Smith;Bob;;;\r\nFN:Bob Smith\r\nORG:Acme\\, Inc.;Sales;\r\nitem1.EMAIL;type=INTERNET;type=pref:bob@acme.test\r\nitem1.X-ABLabel:_$!<Work>!$_\r\nEMAIL;type=INTERNET;type=HOME:bob@home.test\r\nTEL;type=CELL;type=VOICE;type=pref:+1 555 0100\r\nBDAY;X-APPLE-OMIT-YEAR=1604:1604-04-02\r\nNOTE:Met at the\\nconference\\, 2024\r\nPHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRgABAQ\r\n AAAQABAAD\r\nUID:abc-123\r\nEND:VCARD\r\n";

    #[test]
    fn parses_apple_card() {
        let c = parse(APPLE).unwrap();
        assert_eq!(c.uid.as_deref(), Some("abc-123"));
        assert_eq!(c.version.as_deref(), Some("3.0"));
        assert_eq!(c.fields.name, "Bob Smith");
        assert_eq!(c.fields.organization, "Acme, Inc.");
        assert_eq!(
            c.fields.emails,
            vec![
                ContactEmail { email: "bob@acme.test".into(), label: Some("work".into()) },
                ContactEmail { email: "bob@home.test".into(), label: Some("home".into()) },
            ]
        );
        assert_eq!(c.fields.phones, vec![ContactPhone { number: "+1 555 0100".into(), label: Some("cell".into()) }]);
        assert_eq!(c.fields.birthday.as_deref(), Some("--04-02"));
        assert_eq!(c.fields.notes, "Met at the\nconference, 2024");
    }

    #[test]
    fn name_falls_back_to_n_and_v4_values() {
        let c = parse("BEGIN:VCARD\nVERSION:4.0\nN:Doe;Jane;;;\nTEL;VALUE=uri;TYPE=\"voice,home\":tel:+1-555-0101\nBDAY:19900115\nEND:VCARD\n").unwrap();
        assert_eq!(c.fields.name, "Jane Doe");
        assert_eq!(c.fields.phones[0], ContactPhone { number: "+1-555-0101".into(), label: Some("home".into()) });
        assert_eq!(c.fields.birthday.as_deref(), Some("1990-01-15"));
        assert!(parse("BEGIN:VEVENT\nEND:VEVENT\n").is_none());
    }

    #[test]
    fn new_card_round_trips() {
        let f = ContactFields {
            name: "Ana María López".into(),
            emails: vec![ContactEmail { email: "ana@x.test".into(), label: Some("work".into()) }, ContactEmail { email: "ana@home.test".into(), label: None }],
            phones: vec![ContactPhone { number: "+34 600 000 000".into(), label: Some("cell".into()) }],
            organization: "Fábrica; S.L.".into(),
            title: "CTO".into(),
            birthday: Some("1985-07-09".into()),
            notes: "Line one\nLine two, with comma".into(),
        };
        let text = build(None, "u-1", &f);
        assert!(text.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        assert!(text.contains("N:López;Ana María;;;\r\n"));
        assert!(text.lines().all(|l| l.len() <= 76));
        let back = parse(&text).unwrap();
        assert_eq!(back.fields, f);
        assert_eq!(back.uid.as_deref(), Some("u-1"));
    }

    #[test]
    fn patching_keeps_unknown_and_unchanged_properties() {
        let mut f = parse(APPLE).unwrap().fields;
        f.emails.push(ContactEmail { email: "bob@new.test".into(), label: None });
        f.title = "Director".into();
        let text = build(Some(APPLE), "ignored", &f);
        // Untouched: photo (still folded), org line with its sub-unit, birthday, name, uid.
        assert!(text.contains("PHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRgABAQAAAQABAAD"));
        assert!(text.contains("ORG:Acme\\, Inc.;Sales;"));
        assert!(text.contains("BDAY;X-APPLE-OMIT-YEAR=1604:1604-04-02"));
        assert!(text.contains("N:Smith;Bob;;;"));
        assert!(text.contains("UID:abc-123"));
        assert!(!text.contains("UID:ignored"));
        // Emails were rewritten, and the orphaned Apple label went with them.
        assert!(!text.contains("X-ABLabel"));
        assert!(text.contains("EMAIL;TYPE=work:bob@acme.test"));
        assert!(text.contains("EMAIL:bob@new.test"));
        let back = parse(&text).unwrap();
        assert_eq!(back.fields, f);
    }

    #[test]
    fn clearing_a_field_removes_its_line() {
        let mut f = parse(APPLE).unwrap().fields;
        f.notes.clear();
        f.birthday = None;
        let text = build(Some(APPLE), "x", &f);
        assert!(!text.contains("NOTE"));
        assert!(!text.contains("BDAY"));
        assert_eq!(parse(&text).unwrap().fields, f);
    }
}
