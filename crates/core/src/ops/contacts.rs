//! Contact ops: card writes and deletes.

use super::*;

impl Engine {
    pub(crate) async fn run_contact_op(&self, ctx: &SyncCtx, providers: &Providers, op: &OpRow) -> Result<()> {
        let prov = providers.contacts.as_ref().ok_or_else(|| anyhow!("account has no contacts provider"))?;
        let p = &op.payload;
        match op.kind.as_str() {
            "contact_put" => {
                let card_id = p["card_id"].as_str().context("card_id")?;
                // Two tries: the second after merging a server change (412).
                for attempt in 0..2 {
                    let Some(card) = self.store.get_card(card_id)? else { return Ok(()) };
                    let Some(contact) = self.store.get_contact(&card.contact_id)?.filter(|c| !c.deleted) else { return Ok(()) };
                    let Some(book) = self.store.get_address_book(&card.address_book_id)? else { return Ok(()) };
                    let remote = (!card.remote_id.starts_with("local:")).then(|| card.remote_id.clone());
                    let base = satchel_ical::vcard::parse(&card.vcard).map(|c| c.fields);
                    // An edit applies its change to this card's own content
                    // (see `push_cards`); without one, the card gets the contact.
                    let target = match (p.get("base").and_then(|b| serde_json::from_value::<ContactFields>(b.clone()).ok()), &base) {
                        (Some(before), Some(own)) => crate::contacts::merge_fields(&before, &contact.fields, own),
                        _ => contact.fields.clone(),
                    };
                    if remote.is_some() && base.as_ref() == Some(&target) {
                        return Ok(());
                    }
                    let vcard = satchel_ical::vcard::build(Some(&card.vcard), &card.uid, &target);
                    let put = ContactOp::Put { book_remote_id: book.remote_id.clone(), card_remote_id: remote.clone(), etag: card.etag.clone(), uid: card.uid.clone(), vcard: vcard.clone() };
                    match prov.apply(ctx, &put).await {
                        Ok(out) => {
                            let rid = out.card_remote_id.or(remote).unwrap_or(card.remote_id);
                            self.store.set_card_remote(card_id, &rid, out.card_etag.as_deref(), out.card_vcard.as_deref().unwrap_or(&vcard))?;
                            break;
                        }
                        Err(e) if attempt == 0 && remote.is_some() && e.downcast_ref::<ApiError>().is_some_and(|a| a.code == ErrorCode::Conflict) => {
                            // Changed on the server since we last looked: merge, then retry.
                            match prov.fetch(ctx, &book.remote_id, &card.remote_id).await? {
                                Some((etag, text)) => {
                                    let theirs = satchel_ical::vcard::parse(&text).map(|c| c.fields).unwrap_or_default();
                                    // Their change reaches the contact; ours is
                                    // reapplied to their version on the retry.
                                    let merged = crate::contacts::merge_fields(&base.clone().unwrap_or_default(), &contact.fields, &theirs);
                                    if merged != contact.fields {
                                        self.store.update_contact(&contact.id, &merged, None)?;
                                    }
                                    self.store.set_card_remote(card_id, &card.remote_id, etag.as_deref(), &text)?;
                                }
                                // Deleted on the server while edited here: the edit wins, recreate it.
                                None => self.store.set_card_remote(card_id, &format!("local:{}", crate::store::new_id()), None, &card.vcard)?,
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            "contact_delete" => {
                let book = p["book_remote_id"].as_str().context("book_remote_id")?.to_string();
                let remote = p["remote_id"].as_str().context("remote_id")?.to_string();
                let etag = p.get("etag").and_then(Value::as_str).map(String::from);
                let del = |etag| ContactOp::Delete { book_remote_id: book.clone(), card_remote_id: remote.clone(), etag };
                match prov.apply(ctx, &del(etag)).await {
                    Ok(_) => {}
                    // The user asked to delete it; a newer server version doesn't change that.
                    Err(e) if e.downcast_ref::<ApiError>().is_some_and(|a| a.code == ErrorCode::Conflict) => {
                        prov.apply(ctx, &del(None)).await?;
                    }
                    Err(e) => return Err(e),
                }
            }
            other => return Err(anyhow!("unknown contacts op {other}")),
        }
        ctx.contacts_changed();
        self.mirror_wake.notify_one();
        Ok(())
    }
}
