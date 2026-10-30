//! The automatic contacts pass and the user's contact commands.

use super::*;

impl Engine {
    /// Work out who you correspond with and create a contact for each new
    /// correspondent. Idempotent; the first run is the backfill.
    /// Returns the number of contacts created or renamed.
    pub fn contacts_pass(&self) -> Result<usize> {
        let evidence = self.store.correspondence_evidence()?;
        let own: HashSet<String> = self.store.list_accounts()?.into_iter().map(|a| a.email.to_lowercase()).collect();
        let spam: HashSet<String> = self.store.list_sender_rules()?.into_iter().filter(|r| r.verdict == SenderVerdict::Spam).map(|r| r.email).collect();
        let people = correspondents(&evidence, &own, |e| spam.contains(e));
        let claimed = self.store.claimed_emails()?;
        let mut changed = 0;
        for p in &people {
            if claimed.contains(&p.email) {
                continue;
            }
            let fields = ContactFields { name: p.name.clone().unwrap_or_default(), emails: vec![ContactEmail { email: p.email.clone(), label: None }], ..Default::default() };
            self.store.insert_contact(&fields, ContactOrigin::Auto)?;
            changed += 1;
        }
        // Untouched automatic contacts follow the best name seen so far, and
        // go away if the user has since marked them as spam, or if another
        // contact has the address (a server card that arrived while this
        // pass ran, or one the user deleted).
        let names: HashMap<&str, &str> = people.iter().filter_map(|p| Some((p.email.as_str(), p.name.as_deref()?))).collect();
        for c in self.store.auto_contacts()? {
            let [e] = c.fields.emails.as_slice() else { continue };
            let email = e.email.to_lowercase();
            let others = self.store.contacts_by_email(&email)?;
            let superseded = others.first().is_some_and(|o| o.id != c.id)
                || others.iter().any(|o| o.id != c.id && (o.deleted || o.origin != ContactOrigin::Auto));
            if spam.contains(&email) || superseded {
                self.store.delete_contact(&c.id)?;
                changed += 1;
                continue;
            }
            if let Some(n) = names.get(email.as_str())
                && *n != c.fields.name
            {
                let mut f = c.fields.clone();
                f.name = n.to_string();
                self.store.update_contact(&c.id, &f, None)?;
                changed += 1;
            }
        }
        if changed > 0 {
            self.emit(CoreEvent::ContactsChanged);
            self.mirror_wake.notify_one();
        }
        Ok(changed)
    }

    /// The contact any of these addresses belongs to (the first one found),
    /// and all of the addresses: the contact's first, then any others given.
    pub(crate) fn person_addresses(&self, emails: &[String]) -> Result<(Option<StoredContact>, Vec<String>)> {
        let given: Vec<String> = emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
        let mut contact = None;
        for e in &given {
            if let Some(c) = self.store.contacts_by_email(e)?.into_iter().find(|c| !c.deleted) {
                contact = Some(c);
                break;
            }
        }
        let mut all: Vec<String> = contact.iter().flat_map(|c| c.fields.emails.iter().map(email_key)).collect();
        for e in given {
            if !all.contains(&e) {
                all.push(e);
            }
        }
        Ok((contact, all))
    }

    /// Accounts whose address books this caller may see (None = all, and
    /// contacts not tied to any account).
    pub(crate) fn contact_scope(&self, caller: &Caller) -> Result<Option<Vec<String>>> {
        match caller {
            Caller::User => Ok(None),
            Caller::Agent(a) => {
                if !a.policy.allow_contacts {
                    return Err(forbidden("contacts are not shared with this agent"));
                }
                Ok(a.policy.account_ids.clone())
            }
        }
    }

    pub(crate) fn contact_view(&self, c: StoredContact, sources: &HashMap<String, Vec<ContactSource>>, stats: &HashMap<String, (u32, chrono::DateTime<Utc>)>) -> Contact {
        let mut message_count = 0;
        let mut last = None;
        for e in &c.fields.emails {
            if let Some((n, d)) = stats.get(&e.email.to_lowercase()) {
                message_count += n;
                last = last.max(Some(*d));
            }
        }
        Contact { sources: sources.get(&c.id).cloned().unwrap_or_default(), id: Some(c.id), fields: c.fields, origin: c.origin, message_count, last_contacted: last, version: c.version }
    }

    /// Whether a scoped caller may see this contact: only ones saved in an
    /// address book of an allowed account.
    pub(crate) fn in_scope(scope: &Option<Vec<String>>, c: &Contact) -> bool {
        scope.as_ref().is_none_or(|ids| c.sources.iter().any(|s| ids.contains(&s.account_id)))
    }

    pub(crate) fn list_contacts(&self, caller: &Caller, search: Option<&str>, limit: Option<u32>) -> Result<Vec<Contact>> {
        let scope = self.contact_scope(caller)?;
        let sources = self.store.contact_sources()?;
        let stats = self.store.address_stats()?;
        let search = search.map(str::trim).filter(|s| !s.is_empty());
        Ok(self
            .store
            .list_contacts(search, limit.unwrap_or(5000).min(20_000))?
            .into_iter()
            .map(|c| self.contact_view(c, &sources, &stats))
            .filter(|c| Self::in_scope(&scope, c))
            .collect())
    }

    pub(crate) fn get_contact(&self, caller: &Caller, id: &str) -> Result<Contact> {
        let scope = self.contact_scope(caller)?;
        let c = self.store.get_contact(id)?.filter(|c| !c.deleted).ok_or_else(|| not_found("contact"))?;
        let c = self.contact_view(c, &self.store.contact_sources()?, &self.store.address_stats()?);
        if !Self::in_scope(&scope, &c) {
            return Err(not_found("contact"));
        }
        Ok(c)
    }

    pub(crate) fn list_address_books(&self, caller: &Caller) -> Result<Vec<AddressBook>> {
        let scope = self.contact_scope(caller)?;
        Ok(self
            .store
            .list_address_books(None)?
            .into_iter()
            .map(|b| b.book)
            .filter(|b| scope.as_ref().is_none_or(|ids| ids.contains(&b.account_id)))
            .collect())
    }

    /// Push an edit to every address book the contact is in. Each card gets
    /// only the change from `before` to the contact's fields now, applied to
    /// its own content: a person's cards in different books (work, personal)
    /// keep their own numbers and notes.
    pub(crate) fn push_cards(&self, contact_id: &str, before: &ContactFields, actor: &str) -> Result<()> {
        for card in self.store.cards_of_contact(contact_id)? {
            self.enqueue(&card.account_id, "contact_put", json!({ "card_id": card.id, "base": before }), actor)?;
        }
        Ok(())
    }

    /// Save a copy of the contact in a server address book.
    pub(crate) fn add_card(&self, contact_id: &str, fields: &ContactFields, book_id: &str, actor: &str) -> Result<()> {
        let book = self.store.get_address_book(book_id)?.ok_or_else(|| not_found("address book"))?;
        if book.book.read_only {
            return Err(invalid("that address book is read-only"));
        }
        if self.store.cards_of_contact(contact_id)?.iter().any(|c| c.address_book_id == book_id) {
            return Ok(());
        }
        let uid = satchel_ical::vcard::new_uid();
        let vcard = satchel_ical::vcard::build(None, &uid, fields);
        let card = self.store.insert_card(contact_id, book_id, &book.book.account_id, &format!("local:{}", crate::store::new_id()), None, &uid, &vcard)?;
        self.enqueue(&book.book.account_id, "contact_put", json!({ "card_id": card }), actor)?;
        Ok(())
    }

    pub(crate) fn save_contact(&self, contact: Contact, address_book_id: Option<&str>, actor: &str) -> Result<Contact> {
        let fields = normalize(&contact.fields)?;
        let id = match &contact.id {
            Some(id) => {
                let old = self.store.get_contact(id)?.filter(|c| !c.deleted).ok_or_else(|| not_found("contact"))?;
                // An edited automatic contact is the user's now.
                let origin = (old.origin == ContactOrigin::Auto).then_some(ContactOrigin::Local);
                if fields != old.fields || origin.is_some() {
                    self.store.update_contact(id, &fields, origin)?;
                    self.push_cards(id, &old.fields, actor)?;
                }
                id.clone()
            }
            None => self.store.insert_contact(&fields, ContactOrigin::Local)?,
        };
        if let Some(book) = address_book_id {
            self.add_card(&id, &fields, book, actor)?;
        }
        self.store.audit(actor, if contact.id.is_some() { "contact_updated" } else { "contact_created" }, &json!({ "name": fields.name, "emails": fields.emails.iter().map(|e| &e.email).collect::<Vec<_>>() }))?;
        self.contacts_touched();
        self.get_contact(&Caller::User, &id)
    }

    pub(crate) fn delete_contact(&self, id: &str, actor: &str) -> Result<()> {
        let c = self.store.get_contact(id)?.filter(|c| !c.deleted).ok_or_else(|| not_found("contact"))?;
        for card in self.store.cards_of_contact(id)? {
            self.delete_card_remote(&card, actor)?;
        }
        self.store.tombstone_contact(id)?;
        self.store.audit(actor, "contact_deleted", &json!({ "name": c.fields.name, "emails": c.fields.emails.iter().map(|e| &e.email).collect::<Vec<_>>() }))?;
        self.contacts_touched();
        Ok(())
    }

    pub(crate) fn delete_card_remote(&self, card: &crate::store::StoredCard, actor: &str) -> Result<()> {
        if !card.remote_id.starts_with("local:")
            && let Some(book) = self.store.get_address_book(&card.address_book_id)?
        {
            self.enqueue(&card.account_id, "contact_delete", json!({ "book_remote_id": book.remote_id, "remote_id": card.remote_id, "etag": card.etag }), actor)?;
        }
        self.store.delete_card(&card.id)
    }

    /// Merge contacts into the first: one person with all their addresses.
    /// Their server cards are linked to it, not deleted or rewritten: a card
    /// can hold fields Satchel doesn't show (photos, postal addresses), and
    /// deleting it would lose them. Later edits reach every linked card.
    pub(crate) fn merge_contacts(&self, ids: &[String], actor: &str) -> Result<Contact> {
        let mut uniq: Vec<&String> = vec![];
        for id in ids {
            if !uniq.contains(&id) {
                uniq.push(id);
            }
        }
        if uniq.len() < 2 {
            return Err(invalid("choose at least two contacts to merge"));
        }
        let mut all = vec![];
        for id in &uniq {
            all.push(self.store.get_contact(id)?.filter(|c| !c.deleted).ok_or_else(|| not_found("contact"))?);
        }
        let primary = all[0].clone();
        let mut fields = primary.fields.clone();
        for other in &all[1..] {
            fields = union_fields(&fields, &other.fields);
            self.store.relink_cards(&other.id, &primary.id)?;
            self.store.delete_contact(&other.id)?;
        }
        let origin = if self.store.cards_of_contact(&primary.id)?.is_empty() { ContactOrigin::Local } else { ContactOrigin::Remote };
        self.store.update_contact(&primary.id, &fields, Some(origin))?;
        self.store.audit(actor, "contacts_merged", &json!({ "name": fields.name, "merged": uniq.len() }))?;
        self.contacts_touched();
        self.get_contact(&Caller::User, &primary.id)
    }

    pub(crate) fn contacts_touched(&self) {
        self.emit(CoreEvent::ContactsChanged);
        self.mirror_wake.notify_one();
    }

    /// Risk notes for an agent's contact proposal (0023: risky ones wait).
    /// Only contacts `caller` may see count, so a warning can't reveal one
    /// outside the agent's accounts.
    pub(crate) fn contact_proposal_warnings(&self, caller: &Caller, contact: &Contact) -> Result<(Vec<String>, Option<i64>)> {
        let mut warnings = vec![];
        let fields = normalize(&contact.fields)?;
        let version = match &contact.id {
            Some(id) => {
                let old = self.store.get_contact(id)?.filter(|c| !c.deleted).ok_or_else(|| not_found("contact"))?;
                let had: HashSet<String> = old.fields.emails.iter().map(email_key).collect();
                for e in &fields.emails {
                    if !had.contains(&email_key(e)) {
                        warnings.push(format!("Adds {} to {}", e.email, if old.fields.name.is_empty() { "this contact" } else { &old.fields.name }));
                    }
                }
                if !old.fields.name.is_empty() && old.fields.name != fields.name {
                    warnings.push(format!("Renames {} to {}", old.fields.name, fields.name));
                }
                Some(old.version)
            }
            None => {
                if !fields.name.is_empty() && self.list_contacts(caller, Some(&fields.name), Some(50))?.iter().any(|c| c.fields.name.eq_ignore_ascii_case(&fields.name)) {
                    warnings.push(format!("A contact named {} already exists", fields.name));
                }
                None
            }
        };
        Ok((warnings, version))
    }
}
