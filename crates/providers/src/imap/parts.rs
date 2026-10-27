//! Downloading a message without its attachments: read the BODYSTRUCTURE,
//! then fetch every part's headers and only the contents a stored copy keeps
//! (0036).

use super::*;

use async_imap::imap_proto::{BodyStructure, ContentEncoding, MessageSection, SectionPath};
use satchel_mime::{Body, Part, Slim};

/// What to fetch for one part, by its IMAP part number.
#[derive(Debug, PartialEq)]
pub(super) struct Plan {
    path: Vec<u32>,
    kind: Kind,
}

#[derive(Debug, PartialEq)]
enum Kind {
    Multipart(Vec<Plan>),
    Keep,
    /// Left on the server; the decoded size.
    Skip(u64),
}

/// A section of a FETCH response.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Section {
    Header,
    Text,
    Mime(Vec<u32>),
    Body(Vec<u32>),
}

impl Plan {
    pub(super) fn new(bs: &BodyStructure) -> Plan {
        node(bs, vec![], false)
    }

    /// Whether any part's contents can stay on the server.
    pub(super) fn skips_any(&self) -> bool {
        match &self.kind {
            Kind::Multipart(c) => c.iter().any(Plan::skips_any),
            Kind::Keep => false,
            Kind::Skip(_) => true,
        }
    }

    /// The FETCH items for the sections [`Self::part`] needs.
    pub(super) fn fetch_items(&self) -> String {
        let mut items = vec!["UID".to_string()];
        self.sections(&mut items);
        format!("({})", items.join(" "))
    }

    fn sections(&self, out: &mut Vec<String>) {
        let n = self.path.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(".");
        out.push(if self.path.is_empty() { "BODY.PEEK[HEADER]".into() } else { format!("BODY.PEEK[{n}.MIME]") });
        match &self.kind {
            Kind::Multipart(c) => c.iter().for_each(|p| p.sections(out)),
            Kind::Keep => out.push(if self.path.is_empty() { "BODY.PEEK[TEXT]".into() } else { format!("BODY.PEEK[{n}]") }),
            Kind::Skip(_) => {}
        }
    }

    /// The part tree, from the fetched sections.
    pub(super) fn part(&self, get: &dyn Fn(&Section) -> Option<Vec<u8>>) -> Option<Part> {
        let root = self.path.is_empty();
        let mut headers = get(&if root { Section::Header } else { Section::Mime(self.path.clone()) })?;
        if headers.ends_with(b"\r\n\r\n") || headers == b"\r\n" {
            headers.truncate(headers.len() - 2);
        }
        let body = match &self.kind {
            Kind::Multipart(c) => Body::Multipart(c.iter().map(|p| p.part(get)).collect::<Option<_>>()?),
            Kind::Keep => Body::Encoded(get(&if root { Section::Text } else { Section::Body(self.path.clone()) })?),
            Kind::Skip(size) => Body::Skipped(*size),
        };
        Some(Part { headers, body })
    }
}

fn node(bs: &BodyStructure, path: Vec<u32>, in_alt: bool) -> Plan {
    let (common, other) = match bs {
        BodyStructure::Multipart { common, bodies, .. } => {
            let alt = common.ty.subtype.eq_ignore_ascii_case("alternative");
            let kids = bodies
                .iter()
                .enumerate()
                .map(|(i, b)| node(b, path.iter().copied().chain([i as u32 + 1]).collect(), alt))
                .collect();
            return Plan { path, kind: Kind::Multipart(kids) };
        }
        BodyStructure::Basic { common, other, .. } | BodyStructure::Text { common, other, .. } | BodyStructure::Message { common, other, .. } => (common, other),
    };
    let mime = format!("{}/{}", common.ty.ty, common.ty.subtype);
    let size = decoded_size(other.octets as u64, &other.transfer_encoding);
    let disposition = common.disposition.as_ref().map(|d| d.ty.as_ref());
    let keep = satchel_mime::keeps_contents(&mime, disposition, other.id.is_some(), size, in_alt);
    Plan { path, kind: if keep { Kind::Keep } else { Kind::Skip(size) } }
}

/// About how many bytes `octets` of a part's encoded contents decode to:
/// base64 lines are 76 characters (57 bytes) plus a CRLF, except the last.
fn decoded_size(octets: u64, enc: &ContentEncoding) -> u64 {
    match enc {
        ContentEncoding::Base64 => (octets + 2) * 57 / 78,
        _ => octets,
    }
}

/// Look a section up in a FETCH response.
pub(super) fn section_of(f: &Fetch, s: &Section) -> Option<Vec<u8>> {
    match s {
        Section::Header => f.header(),
        Section::Text => f.text(),
        Section::Mime(p) => f.section(&SectionPath::Part(p.clone(), Some(MessageSection::Mime))),
        Section::Body(p) => f.section(&SectionPath::Part(p.clone(), None)),
    }
    .map(<[u8]>::to_vec)
}

impl ImapProvider {
    /// The message with UID `uid` in the selected mailbox, without
    /// attachment contents if `plan` leaves any out. `None`: the server
    /// didn't send what was asked for, or the result doesn't rebuild.
    pub(super) async fn fetch_slim(&self, s: &mut Session, uid: u32, plan: &Plan) -> Result<Option<Slim>> {
        let fetches: Vec<Fetch> = s.uid_fetch(uid.to_string(), plan.fetch_items()).await?.try_collect().await?;
        let Some(f) = fetches.iter().find(|f| f.uid == Some(uid)) else { return Ok(None) };
        Ok(plan.part(&|sec| section_of(f, sec)).and_then(|p| satchel_mime::rebuild(&p)))
    }

    /// The plans of the messages with these UIDs that are big enough and
    /// have contents to leave on the server.
    pub(super) async fn plans(&self, s: &mut Session, uids: &str) -> Result<HashMap<u32, Plan>> {
        let fetches: Vec<Fetch> = s.uid_fetch(uids, "(UID RFC822.SIZE BODYSTRUCTURE)").await?.try_collect().await?;
        Ok(fetches
            .iter()
            .filter(|f| f.size.is_none_or(|n| n as u64 >= satchel_mime::SLIM_FROM_BYTES))
            .filter_map(|f| Some((f.uid?, Plan::new(f.bodystructure()?))))
            .filter(|(_, p)| p.skips_any())
            .collect())
    }
}
