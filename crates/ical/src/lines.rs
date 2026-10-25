//! Content lines: unfolding, parsing, escaping and folding (shared with vCard).


/// One property line, after unfolding.
#[derive(Debug, Clone)]
pub(crate) struct Prop {
    pub(crate) name: String,
    pub(crate) params: Vec<(String, String)>,
    pub(crate) value: String,
}

impl Prop {
    pub(crate) fn param(&self, k: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
}

pub(crate) fn unfold(s: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for raw in s.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

pub(crate) fn parse_line(line: &str) -> Option<Prop> {
    // Split name;params:value — colons may appear inside quoted params.
    let mut in_quote = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ':' if !in_quote => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = split_unquoted(head, ';');
    let name = parts.remove(0).to_ascii_uppercase();
    let params = parts
        .into_iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
        })
        .collect();
    Some(Prop { name, params, value: value.to_string() })
}

pub(crate) fn split_unquoted(s: &str, sep: char) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q = false;
    for c in s.chars() {
        if c == '"' {
            q = !q;
        }
        if c == sep && !q {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}


pub(crate) fn fold(line: &str) -> String {
    // Fold at 74 octets on char boundaries.
    let mut out = String::new();
    let mut count = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if count + len > 74 {
            out.push_str("\r\n ");
            count = 1;
        }
        out.push(c);
        count += len;
    }
    out.push_str("\r\n");
    out
}
