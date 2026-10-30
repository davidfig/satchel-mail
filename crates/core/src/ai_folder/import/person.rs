//! Person files (`contacts/people/`) → contact proposals.

use super::*;

/// Read a file in `contacts/people/`. `last` is what the mirror last wrote
/// there, if it remembers. Returns None when nothing changed, the proposal an
/// added or edited file amounts to, or an error to report back.
///
/// An edit only proposes the header fields the agent changed (a three-way
/// merge against what it read), so changes made since then are kept. Without
/// `last` (after a restart), a file is taken as edited only if it is at the
/// contact's current version; an older one is just out of date.
pub fn read_person(store: &Store, text: &str, last: Option<&str>) -> Result<Option<DroppedProposal>> {
    if last == Some(text) {
        return Ok(None);
    }
    let (fm, body) = parse(text)?;
    // Checked on the header only: notes exported from a contact may mention
    // anything, and aren't the agent's to answer for.
    if fm.get("agent_token").is_some() {
        bail!(TOKEN_REFUSED);
    }
    let contact = match fm.str("contact_id") {
        None => {
            let fields = person_fields(&fm, &body, true)?;
            if fields.name.trim().is_empty() && fields.emails.is_empty() {
                bail!("give `name:` or `emails:` for a new contact");
            }
            Contact { fields, ..Default::default() }
        }
        Some(id) => {
            // One outside the folder's accounts reads as gone, like a deleted one.
            let Some(current) = contact_by_id(store, id)? else {
                if last.is_none() {
                    return Ok(None);
                }
                bail!("contact {id} no longer exists");
            };
            let base_text = match last {
                Some(l) => l.to_string(),
                None if fm.str("version").and_then(|v| v.parse().ok()) == Some(current.version) => crate::ai_folder::contact_file(
                    &Contact { id: Some(current.id.clone()), fields: current.fields.clone(), version: current.version, ..Default::default() },
                    &Default::default(),
                ),
                None => return Ok(None),
            };
            let (bfm, bbody) = parse(&base_text)?;
            let mut base = person_fields(&bfm, &bbody, false)?;
            let mut theirs = person_fields(&fm, &body, false)?;
            if base == theirs {
                if last.is_none() {
                    return Ok(None);
                }
                bail!("nothing to change: edit the fields in the header between the `---` lines (the list below them is only for reading)");
            }
            // The header has no labels; keep the ones the contact has.
            for f in [&mut base, &mut theirs] {
                for e in &mut f.emails {
                    e.label = current.fields.emails.iter().find(|x| x.email.eq_ignore_ascii_case(&e.email)).and_then(|x| x.label.clone());
                }
                for p in &mut f.phones {
                    p.label = current.fields.phones.iter().find(|x| x.number == p.number).and_then(|x| x.label.clone());
                }
            }
            let fields = crate::contacts::merge_fields(&base, &current.fields, &theirs);
            if fields == current.fields {
                bail!("nothing to change: the contact already has these values");
            }
            Contact { id: Some(current.id), fields, ..Default::default() }
        }
    };
    let why = fm.str("why").or(fm.str("rationale")).unwrap_or_default().to_string();
    Ok(Some(DroppedProposal {
        agent_token: fm.str("agent_token").map(str::to_string),
        agent: fm.str("agent").map(str::to_string),
        action: ProposalAction::SaveContact { contact },
        rationale: why,
        source_thread_ids: vec![],
    }))
}

/// The editable fields of a person file. Notes are the text under
/// `## Notes`; a new file without that heading uses its whole body.
pub(crate) fn person_fields(fm: &FrontMatter, body: &str, new: bool) -> Result<ContactFields> {
    let notes = match body.split_once("\n## Notes\n").or_else(|| body.strip_prefix("## Notes\n").map(|n| ("", n))) {
        Some((_, n)) => n.trim().to_string(),
        None if new => body.trim().strip_prefix("# ").map(|b| b.split_once('\n').map(|(_, r)| r).unwrap_or("")).unwrap_or(body.trim()).trim().to_string(),
        None => String::new(),
    };
    let text = |k: &str| fm.str(k).unwrap_or_default().to_string();
    Ok(ContactFields {
        name: text("name"),
        emails: fm.list("emails").iter().chain(fm.list("email").iter()).map(|s| Ok(ContactEmail { email: address(s)?.email, label: None })).collect::<Result<_>>()?,
        phones: fm.list("phones").into_iter().chain(fm.list("phone")).map(|number| ContactPhone { number, label: None }).collect(),
        organization: text("organization"),
        title: text("title"),
        birthday: fm.str("birthday").map(str::to_string),
        notes,
    })
}
