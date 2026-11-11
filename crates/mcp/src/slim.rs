//! Keeping responses small: text bodies, truncation.

use super::*;

pub(crate) fn truncate(s: &str) -> String {
    if s.chars().count() > MAX_BODY_CHARS {
        format!("{}\n\n[truncated]", s.chars().take(MAX_BODY_CHARS).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Agents get text bodies, not HTML, to keep responses small, and cleaned
/// of tracking and padding (0075). A single message keeps its quoted
/// history; a thread's messages show only what each added.
pub(crate) fn slim_message(m: Value) -> Value {
    slim(m, false)
}

pub(crate) fn slim_thread(mut t: Value) -> Value {
    if let Some(msgs) = t.get_mut("messages").and_then(Value::as_array_mut) {
        for m in msgs.iter_mut() {
            *m = slim(m.take(), true);
        }
    }
    t
}

fn slim(mut m: Value, strip_quotes: bool) -> Value {
    if let Some(o) = m.as_object_mut() {
        let html = o.remove("html_body");
        o.remove("remote_images_blocked");
        o.remove("references");
        if let Some(Value::String(t)) = o.get("text_body") {
            let source = satchel_mime::agent_source(t, html.as_ref().and_then(Value::as_str));
            let text = if strip_quotes { satchel_mime::strip_quotes(&source) } else { source.into_owned() };
            o.insert("text_body".into(), Value::String(truncate(&satchel_mime::agent_text(&text))));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_cleaned_for_agents() {
        let m = json!({
            "text_body": "Your order shipped: https://shop.test/o/1?utm_source=email\n\n\n\nOn Mon, Bob wrote:\n> earlier",
            "html_body": "<p>html</p>",
            "references": ["<a@b>"],
        });
        let one = slim_message(m.clone());
        assert_eq!(one["text_body"], "Your order shipped: https://shop.test/o/1\n\nOn Mon, Bob wrote:\n> earlier");
        assert!(one.get("html_body").is_none() && one.get("references").is_none());
        let thread = slim_thread(json!({ "messages": [m] }));
        assert_eq!(thread["messages"][0]["text_body"], "Your order shipped: https://shop.test/o/1", "a thread shows what each message added");
    }
}
