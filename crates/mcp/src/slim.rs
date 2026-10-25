//! Keeping responses small: text bodies, truncation.

use super::*;

pub(crate) fn truncate(s: &str) -> String {
    if s.chars().count() > MAX_BODY_CHARS {
        format!("{}\n\n[truncated]", s.chars().take(MAX_BODY_CHARS).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Agents get text bodies, not HTML, to keep responses small.
pub(crate) fn slim_message(mut m: Value) -> Value {
    if let Some(o) = m.as_object_mut() {
        o.remove("html_body");
        o.remove("remote_images_blocked");
        o.remove("references");
        if let Some(Value::String(t)) = o.get("text_body") {
            let t = truncate(t);
            o.insert("text_body".into(), Value::String(t));
        }
    }
    m
}

pub(crate) fn slim_thread(mut t: Value) -> Value {
    if let Some(msgs) = t.get_mut("messages").and_then(Value::as_array_mut) {
        for m in msgs.iter_mut() {
            let mut slim = slim_message(m.take());
            // Collapse quoted history; the earlier messages are in the thread already.
            if let Some(Value::String(txt)) = slim.get("text_body") {
                let s = satchel_mime_strip(txt);
                slim["text_body"] = Value::String(s);
            }
            *m = slim;
        }
    }
    t
}

pub(crate) fn satchel_mime_strip(s: &str) -> String {
    let mut out = vec![];
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with("On ") && l.ends_with("wrote:") {
            break;
        }
        if l.starts_with('>') {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}
