//! Message ids, folder names (modified UTF-7), roles, flags and UID sets.

use super::*;

pub fn remote_id(uidvalidity: u32, uid: u32, mailbox: &str) -> String {
    format!("{uidvalidity}:{uid}:{mailbox}")
}

pub fn parse_remote_id(s: &str) -> Option<(u32, u32, &str)> {
    let mut it = s.splitn(3, ':');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?))
}

/// Decode IMAP modified UTF-7 (RFC 3501 §5.1.3).
pub fn decode_mutf7(s: &str) -> String {
    use base64::Engine as _;
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('-') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let enc = &after[..j];
        if enc.is_empty() {
            out.push('&');
        } else {
            let b64 = enc.replace(',', "/");
            let pad = (4 - b64.len() % 4) % 4;
            let b64 = format!("{b64}{}", "=".repeat(pad));
            match base64::engine::general_purpose::STANDARD.decode(b64) {
                Ok(bytes) => {
                    let units: Vec<u16> = bytes.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                Err(_) => out.push_str(&rest[i..i + 2 + j]),
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

/// Encode IMAP modified UTF-7 (RFC 3501 §5.1.3).
pub fn encode_mutf7(s: &str) -> String {
    use base64::Engine as _;
    let mut out = String::new();
    let mut run: Vec<u16> = vec![];
    let flush = |run: &mut Vec<u16>, out: &mut String| {
        if run.is_empty() {
            return;
        }
        let bytes: Vec<u8> = run.iter().flat_map(|u| u.to_be_bytes()).collect();
        let b64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes).replace('/', ",");
        out.push('&');
        out.push_str(&b64);
        out.push('-');
        run.clear();
    };
    for c in s.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut run, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            run.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut run, &mut out);
    out
}

pub(super) fn role_for(name: &str, attrs: &[NameAttribute<'_>]) -> MailboxRole {
    for a in attrs {
        match a {
            NameAttribute::Sent => return MailboxRole::Sent,
            NameAttribute::Drafts => return MailboxRole::Drafts,
            NameAttribute::Trash => return MailboxRole::Trash,
            NameAttribute::Junk => return MailboxRole::Junk,
            NameAttribute::Archive => return MailboxRole::Archive,
            NameAttribute::All => return MailboxRole::All,
            NameAttribute::Flagged => return MailboxRole::Flagged,
            _ => {}
        }
    }
    let leaf = name.rsplit(['/', '.']).next().unwrap_or(name).to_lowercase();
    if name.eq_ignore_ascii_case("INBOX") {
        return MailboxRole::Inbox;
    }
    match leaf.as_str() {
        "sent" | "sent items" | "sent mail" | "sent messages" => MailboxRole::Sent,
        "drafts" | "draft" => MailboxRole::Drafts,
        "trash" | "deleted items" | "deleted messages" | "bin" => MailboxRole::Trash,
        "junk" | "spam" | "junk mail" | "junk e-mail" | "bulk mail" => MailboxRole::Junk,
        "archive" | "archives" => MailboxRole::Archive,
        _ => MailboxRole::Custom,
    }
}

pub(super) fn flags_of(f: &Fetch) -> (Flags, bool) {
    let mut out = Flags::default();
    let mut deleted = false;
    for fl in f.flags() {
        match fl {
            Flag::Seen => out.seen = true,
            Flag::Answered => out.answered = true,
            Flag::Flagged => out.flagged = true,
            Flag::Draft => out.draft = true,
            Flag::Deleted => deleted = true,
            _ => {}
        }
    }
    (out, deleted)
}

pub(super) fn uid_set(uids: &[u32]) -> String {
    let mut v = uids.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut parts = vec![];
    let mut i = 0;
    while i < v.len() {
        let start = v[i];
        let mut end = start;
        while i + 1 < v.len() && v[i + 1] == end + 1 {
            i += 1;
            end = v[i];
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
        i += 1;
    }
    parts.join(",")
}

pub(super) fn imap_date(d: DateTime<Utc>) -> String {
    d.format("%d-%b-%Y").to_string()
}
