//! The address book: one file per person, and CONTACTS.md.

use super::*;

pub(crate) fn display_name(f: &ContactFields) -> String {
    if !f.name.is_empty() {
        f.name.clone()
    } else if let Some(e) = f.emails.first() {
        e.email.clone()
    } else if let Some(p) = f.phones.first() {
        p.number.clone()
    } else {
        "(no name)".into()
    }
}

/// `<name>-<last 8 of id>.md`: readable, and stable unless the name changes.
pub(crate) fn contact_file_name(c: &Contact) -> String {
    let id = c.id.as_deref().unwrap_or_default();
    format!("{}-{}.md", slug(&display_name(&c.fields)), &id[id.len().saturating_sub(8)..])
}

/// "me@gmail.com › Contacts"
pub(crate) fn source_label(s: &ContactSource, accounts: &HashMap<String, String>) -> String {
    format!("{} › {}", accounts.get(&s.account_id).map(String::as_str).unwrap_or("(removed account)"), s.name)
}

/// A value as JSON, which is also valid YAML.
pub(crate) fn j<T: serde::Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// One person's file: a `---` header with every field (values in JSON), then the same for reading, then the notes.
pub(crate) fn contact_file(c: &Contact, accounts: &HashMap<String, String>) -> String {
    let f = &c.fields;
    let mut acc_emails: Vec<String> = vec![];
    for s in &c.sources {
        let e = accounts.get(&s.account_id).cloned().unwrap_or_default();
        if !e.is_empty() && !acc_emails.contains(&e) {
            acc_emails.push(e);
        }
    }
    let books: Vec<serde_json::Value> = c
        .sources
        .iter()
        .map(|s| serde_json::json!({ "account": accounts.get(&s.account_id), "address_book": s.name, "address_book_id": s.address_book_id }))
        .collect();
    let origin = match c.origin {
        ContactOrigin::Auto => "auto",
        ContactOrigin::Local => "local",
        ContactOrigin::Remote => "remote",
    };
    let mut s = String::from("---\n");
    s.push_str(&format!("contact_id: {}\n", c.id.as_deref().unwrap_or_default()));
    s.push_str(&format!("name: {}\n", j(&f.name)));
    s.push_str(&format!("emails: {}\n", j(&f.emails.iter().map(|e| e.email.as_str()).collect::<Vec<_>>())));
    s.push_str(&format!("phones: {}\n", j(&f.phones.iter().map(|p| p.number.as_str()).collect::<Vec<_>>())));
    for (k, v) in [("organization", &f.organization), ("title", &f.title)] {
        if !v.is_empty() {
            s.push_str(&format!("{k}: {}\n", j(v)));
        }
    }
    if let Some(b) = &f.birthday {
        s.push_str(&format!("birthday: {}\n", j(b)));
    }
    s.push_str(&format!("origin: {origin}\n"));
    s.push_str(&format!("accounts: {}\n", j(&acc_emails)));
    s.push_str(&format!("address_books: {}\n", j(&books)));
    s.push_str(&format!("messages: {}\n", c.message_count));
    if let Some(d) = c.last_contacted {
        s.push_str(&format!("last_contacted: {}\n", d.to_rfc3339()));
    }
    s.push_str(&format!("version: {}\n---\n\n# {}\n\n", c.version, one_line(&display_name(f))));
    let labeled = |v: &str, l: &Option<String>| match l {
        Some(l) => format!("{} ({})", one_line(v), one_line(l)),
        None => one_line(v),
    };
    for e in &f.emails {
        s.push_str(&format!("- email: {}\n", labeled(&e.email, &e.label)));
    }
    for p in &f.phones {
        s.push_str(&format!("- phone: {}\n", labeled(&p.number, &p.label)));
    }
    let work = [f.title.as_str(), f.organization.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
    if !work.is_empty() {
        s.push_str(&format!("- work: {}\n", one_line(&work)));
    }
    if let Some(b) = &f.birthday {
        s.push_str(&format!("- birthday: {}\n", one_line(b)));
    }
    match c.sources.len() {
        0 if c.origin == ContactOrigin::Auto => s.push_str("- saved in: only in Satchel, added because the user wrote to them\n"),
        0 => s.push_str("- saved in: only in Satchel, not in any account's address book\n"),
        _ => s.push_str(&format!("- saved in: {}\n", one_line(&c.sources.iter().map(|x| source_label(x, accounts)).collect::<Vec<_>>().join("; ")))),
    }
    if let Some(d) = c.last_contacted {
        s.push_str(&format!("- messages: {}, last {}\n", c.message_count, d.format("%Y-%m-%d")));
    }
    if !f.notes.trim().is_empty() {
        s.push_str(&format!("\n## Notes\n\n{}\n", f.notes.trim()));
    }
    s
}

impl Mirror<'_> {
    /// The address book: one Markdown file per person in `contacts/people/`,
    /// and `CONTACTS.md` listing them all. When the folder is limited to some
    /// accounts, only contacts saved in those accounts' address books are
    /// included, and only those address books are named.
    pub fn write_contacts(&self) -> Result<()> {
        let base = self.root().join("contacts");
        let people = base.join("people");
        let accounts: HashMap<String, String> = self.store.list_accounts()?.into_iter().map(|a| (a.id, a.email)).collect();
        let sources = self.store.contact_sources()?;
        // Message counts under the date limit count only the messages inside it.
        let stats = match self.cutoff() {
            Some(c) => self.store.address_stats_since(c)?,
            None => self.store.address_stats()?,
        };
        let mut md = String::from(
            "# Contacts\n\nPeople saved in the user's address books, and people Satchel added because the user has exchanged mail with them recently. Each has a file in `people/` with every field and the address books it is saved in; people Satchel added who haven't written lately are only there.\n\n| name | emails | phones | organization | messages | contact_id |\n|---|---|---|---|---|---|\n",
        );
        let mut listed = HashSet::new();
        let mut written = self.contact_files.lock().unwrap();
        for c in self.store.list_contacts(None, 50_000)? {
            let mut srcs = sources.get(&c.id).cloned().unwrap_or_default();
            if let Some(ids) = &self.settings.account_ids {
                srcs.retain(|s| ids.contains(&s.account_id));
                if srcs.is_empty() {
                    continue;
                }
            }
            let (mut n, mut last) = (0u32, None);
            for e in &c.fields.emails {
                if let Some((m, d)) = stats.get(&e.email.to_lowercase()) {
                    n += m;
                    last = last.max(Some(*d));
                }
            }
            let contact = Contact { id: Some(c.id), fields: c.fields, origin: c.origin, sources: srcs, message_count: n, last_contacted: last, version: c.version };
            let file = contact_file_name(&contact);
            let path = people.join(&file);
            // An agent's edit waiting to be read as a proposal stays as it is.
            let on_disk = read_regular(&self.root(), &path).ok();
            let pending = on_disk.as_deref().is_some_and(|t| !matches!(crate::ai_folder::import::read_person(self.store, t, written.get(&file).map(String::as_str)), Ok(None)));
            if !pending {
                let text = contact_file(&contact, &accounts);
                match write_if_changed(&self.root(), &path, text.as_bytes()) {
                    Ok(()) => {
                        written.insert(file.clone(), text);
                    }
                    Err(e) => tracing::warn!("AI folder: {file}: {e:#}"),
                }
            }
            listed.insert(file.clone());
            // Most automatic contacts are people from years ago; listing them
            // all made this file too big to read whole.
            if contact.origin == ContactOrigin::Auto && contact.message_count == 0 {
                continue;
            }
            let f = &contact.fields;
            let cell = |s: &str| one_line(s).replace('|', "/");
            md.push_str(&format!(
                "| [{}](people/{}) | {} | {} | {} | {} | `{}` |\n",
                cell(&display_name(f)).replace(['[', ']'], ""),
                file,
                cell(&f.emails.iter().map(|e| e.email.as_str()).collect::<Vec<_>>().join(", ")),
                cell(&f.phones.iter().map(|p| p.number.as_str()).collect::<Vec<_>>().join(", ")),
                cell(&[f.title.as_str(), f.organization.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ")),
                contact.message_count,
                contact.id.as_deref().unwrap_or_default()
            ));
        }
        write_if_changed(&self.root(), &base.join("CONTACTS.md"), md.as_bytes())?;
        // Files of deleted contacts and old names of renamed ones go; files an
        // agent added or edited stay until they're read as proposals.
        if let Ok(files) = list_files(&self.root(), &people) {
            for path in files {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if listed.contains(&name) {
                    continue;
                }
                let Ok(text) = read_regular(&self.root(), &path) else { continue };
                let ours = match written.get(&name) {
                    Some(last) => *last == text,
                    None => crate::ai_folder::import::parse(&text).is_ok_and(|(fm, _)| fm.str("contact_id").is_some()),
                };
                if ours {
                    let _ = remove_file(&self.root(), &path);
                    written.remove(&name);
                }
            }
        }
        // Replaced by the per-person files.
        let _ = remove_file(&self.root(), &base.join("contacts.json"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contact_file_names_every_address_book_it_is_in() {
        let accounts = HashMap::from([("g".to_string(), "me@gmail.com".to_string()), ("h".to_string(), "me@hotmail.com".to_string())]);
        let src = |acc: &str, book: &str| ContactSource { address_book_id: format!("{acc}-book"), account_id: acc.into(), name: book.into() };
        let mut c = Contact {
            id: Some("0190aaaabbbbccccdddd1234abcd5678".into()),
            fields: ContactFields {
                name: "Bob \"Bobby\" Smith".into(),
                emails: vec![ContactEmail { email: "bob@x.test".into(), label: Some("work".into()) }],
                notes: "Likes tea".into(),
                ..Default::default()
            },
            origin: ContactOrigin::Remote,
            sources: vec![src("g", "Contacts"), src("h", "Contacts"), src("h", "Family")],
            ..Default::default()
        };
        assert_eq!(contact_file_name(&c), "bob-bobby-smith-abcd5678.md");
        let s = contact_file(&c, &accounts);
        assert!(s.contains("name: \"Bob \\\"Bobby\\\" Smith\"\n"), "{s}");
        assert!(s.contains("accounts: [\"me@gmail.com\",\"me@hotmail.com\"]\n"), "{s}");
        assert!(s.contains("- saved in: me@gmail.com › Contacts; me@hotmail.com › Contacts; me@hotmail.com › Family\n"), "{s}");
        assert!(s.contains("- email: bob@x.test (work)\n") && s.ends_with("## Notes\n\nLikes tea\n"), "{s}");

        c.sources.clear();
        c.origin = ContactOrigin::Auto;
        let s = contact_file(&c, &accounts);
        assert!(s.contains("accounts: []\n") && s.contains("only in Satchel, added because the user wrote to them"), "{s}");
    }
}
