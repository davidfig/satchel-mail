//! Server cards, as sync reports them (called through SyncCtx).

use super::*;

pub(crate) fn parsed_fields(vcard: &str) -> ContactFields {
    satchel_ical::vcard::parse(vcard).map(|c| c.fields).unwrap_or_default()
}

/// Store a card the server reported. Returns whether anything changed.
pub(crate) fn absorb_card(store: &Store, account_id: &str, book_id: &str, remote_id: &str, etag: Option<&str>, vcard: &str) -> Result<bool> {
    let Some(parsed) = satchel_ical::vcard::parse(vcard) else { return Ok(false) };
    let theirs = normalize(&parsed.fields).unwrap_or(parsed.fields.clone());
    let existing = match store.card_by_remote(book_id, remote_id)? {
        Some(c) => Some(c),
        // The server's copy of a card we created and haven't heard back about.
        None => match &parsed.uid {
            Some(uid) => store.pending_card_by_uid(book_id, uid)?,
            None => None,
        },
    };
    if let Some(card) = existing {
        if card.remote_id == remote_id && etag.is_some() && card.etag.as_deref() == etag {
            return Ok(false);
        }
        if let Some(contact) = store.get_contact(&card.contact_id)? {
            let merged = merge_fields(&parsed_fields(&card.vcard), &contact.fields, &theirs);
            if merged != contact.fields {
                store.update_contact(&contact.id, &merged, None)?;
            }
        }
        store.set_card_remote(&card.id, remote_id, etag, vcard)?;
        return Ok(true);
    }
    // New to us. One person may have cards in several address books: link it
    // to a live contact that shares an address and has no card in this book.
    let mut target: Option<StoredContact> = None;
    'find: for e in &theirs.emails {
        for c in store.contacts_by_email(&e.email)? {
            if !c.deleted && !store.cards_of_contact(&c.id)?.iter().any(|k| k.address_book_id == book_id) {
                target = Some(c);
                break 'find;
            }
        }
    }
    let uid = parsed.uid.clone().unwrap_or_else(|| remote_id.to_string());
    let contact_id = match target {
        Some(c) if c.origin == ContactOrigin::Auto => {
            // The card is something the user wrote; it replaces the guess.
            store.update_contact(&c.id, &union_fields(&theirs, &c.fields), Some(ContactOrigin::Remote))?;
            c.id
        }
        Some(c) => {
            let fields = union_fields(&c.fields, &theirs);
            if fields != c.fields {
                store.update_contact(&c.id, &fields, None)?;
            }
            c.id
        }
        None => store.insert_contact(&theirs, ContactOrigin::Remote)?,
    };
    store.insert_card(&contact_id, book_id, account_id, remote_id, etag, &uid, vcard)?;
    Ok(true)
}

/// The server deleted a card. The person goes too unless another address
/// book still has them; it stays as a tombstone so mail doesn't recreate it.
pub(crate) fn card_gone(store: &Store, book_id: &str, remote_id: &str) -> Result<()> {
    let Some(card) = store.card_by_remote(book_id, remote_id)? else { return Ok(()) };
    store.delete_card(&card.id)?;
    if store.cards_of_contact(&card.contact_id)?.is_empty() {
        store.tombstone_contact(&card.contact_id)?;
    }
    Ok(())
}
