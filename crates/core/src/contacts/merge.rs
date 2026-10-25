//! Merging contact fields: three-way against the server's last version, and pooling (pure).

use super::*;

pub(crate) fn email_key(e: &ContactEmail) -> String {
    e.email.trim().to_lowercase()
}

pub(crate) fn phone_key(p: &ContactPhone) -> String {
    let digits: String = p.number.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() { p.number.trim().to_lowercase() } else { digits }
}

/// Three-way merge of a list: start from ours, apply what they added and
/// removed relative to base; label changes from them apply where we kept base's.
pub(crate) fn merge_list<T: Clone + PartialEq>(base: &[T], ours: &[T], theirs: &[T], key: impl Fn(&T) -> String) -> Vec<T> {
    let bk: HashMap<String, &T> = base.iter().map(|x| (key(x), x)).collect();
    let tk: HashMap<String, &T> = theirs.iter().map(|x| (key(x), x)).collect();
    let mut out: Vec<T> = vec![];
    for o in ours {
        let k = key(o);
        match (bk.get(&k), tk.get(&k)) {
            // They removed it and we hadn't touched it.
            (Some(b), None) if *b == o => continue,
            // They changed it (e.g. its label) and we hadn't.
            (Some(b), Some(t)) if *b == o => out.push((*t).clone()),
            _ => out.push(o.clone()),
        }
    }
    for t in theirs {
        let k = key(t);
        if !bk.contains_key(&k) && !out.iter().any(|x| key(x) == k) {
            out.push(t.clone());
        }
    }
    out
}

/// Three-way merge: `base` is what the server last confirmed, `ours` the
/// contact as it is here, `theirs` the server's new version. Where we changed
/// a field, ours wins; otherwise theirs.
pub fn merge_fields(base: &ContactFields, ours: &ContactFields, theirs: &ContactFields) -> ContactFields {
    fn pick<T: Clone + PartialEq>(b: &T, o: &T, t: &T) -> T {
        if o == b { t.clone() } else { o.clone() }
    }
    ContactFields {
        name: pick(&base.name, &ours.name, &theirs.name),
        emails: merge_list(&base.emails, &ours.emails, &theirs.emails, email_key),
        phones: merge_list(&base.phones, &ours.phones, &theirs.phones, phone_key),
        organization: pick(&base.organization, &ours.organization, &theirs.organization),
        title: pick(&base.title, &ours.title, &theirs.title),
        birthday: pick(&base.birthday, &ours.birthday, &theirs.birthday),
        notes: pick(&base.notes, &ours.notes, &theirs.notes),
    }
}

/// Combine two descriptions of one person: `primary`'s values win, `other`
/// fills the gaps; addresses and numbers are pooled; notes are kept from both.
pub fn union_fields(primary: &ContactFields, other: &ContactFields) -> ContactFields {
    let or = |a: &String, b: &String| if a.trim().is_empty() { b.clone() } else { a.clone() };
    let mut emails = primary.emails.clone();
    for e in &other.emails {
        if !emails.iter().any(|x| email_key(x) == email_key(e)) {
            emails.push(e.clone());
        }
    }
    let mut phones = primary.phones.clone();
    for p in &other.phones {
        if !phones.iter().any(|x| phone_key(x) == phone_key(p)) {
            phones.push(p.clone());
        }
    }
    let notes = match (primary.notes.trim(), other.notes.trim()) {
        ("", b) => b.to_string(),
        (a, b) if b.is_empty() || a == b || a.contains(b) => a.to_string(),
        (a, b) => format!("{a}\n\n{b}"),
    };
    ContactFields {
        name: or(&primary.name, &other.name),
        emails,
        phones,
        organization: or(&primary.organization, &other.organization),
        title: or(&primary.title, &other.title),
        birthday: primary.birthday.clone().or_else(|| other.birthday.clone()),
        notes,
    }
}

/// Trim, drop empties and duplicates, check addresses and the birthday.
pub fn normalize(f: &ContactFields) -> Result<ContactFields> {
    let mut out = ContactFields {
        name: f.name.split_whitespace().collect::<Vec<_>>().join(" "),
        organization: f.organization.trim().to_string(),
        title: f.title.trim().to_string(),
        notes: f.notes.trim().to_string(),
        ..Default::default()
    };
    let label = |l: &Option<String>| l.as_deref().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    for e in &f.emails {
        let email = e.email.trim().trim_start_matches("mailto:").to_string();
        if email.is_empty() {
            continue;
        }
        if !valid_address(&email) {
            return Err(invalid(format!("not an email address: {email}")));
        }
        if !out.emails.iter().any(|x| x.email.eq_ignore_ascii_case(&email)) {
            out.emails.push(ContactEmail { email, label: label(&e.label) });
        }
    }
    for p in &f.phones {
        let number = p.number.trim().to_string();
        if !number.is_empty() && !out.phones.iter().any(|x| phone_key(x) == phone_key(p)) {
            out.phones.push(ContactPhone { number, label: label(&p.label) });
        }
    }
    out.birthday = match f.birthday.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        None => None,
        Some(b) => Some(satchel_ical::vcard::normalize_birthday(b, None).ok_or_else(|| invalid("birthday: use YYYY-MM-DD, or --MM-DD without a year"))?),
    };
    if out.name.is_empty() && out.emails.is_empty() {
        return Err(invalid("a contact needs a name or an email address"));
    }
    Ok(out)
}
