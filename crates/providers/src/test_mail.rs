//! A message with attachments, for the tests of downloading without them.

use satchel_api::Attachment;

/// Lines of base64 in the PDF attachment (76 characters each).
const PDF_LINES: usize = 3000;

pub(crate) const PNG_B64: &str = "iVBORw0KGgo=";

/// The full message: text and HTML alternatives, a small inline image, a
/// PDF attachment and a calendar invitation.
pub(crate) fn sample() -> Vec<u8> {
    let pdf = vec!["QUFB".repeat(19); PDF_LINES].join("\r\n");
    format!(
        "From: Bob <bob@x.test>\r\nTo: a@x.test\r\nSubject: Report\r\nMessage-ID: <r1@x.test>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
         --b1\r\nContent-Type: multipart/alternative; boundary=\"b2\"\r\n\r\n\
         --b2\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nCaf=E9 report\r\n\
         --b2\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Caf\u{e9} <img src=\"cid:logo\"></p>\r\n--b2--\r\n\
         --b1\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\n{PNG_B64}\r\n\
         --b1\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n\
         --b1\r\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b1--\r\n"
    )
    .into_bytes()
}

/// The PDF's size once decoded.
pub(crate) fn pdf_size() -> u64 {
    (PDF_LINES * 57) as u64
}

/// `slim` reads like the full message, keeps what a stripped copy keeps, and
/// is small.
pub(crate) fn assert_slim(full: &[u8], raw: &[u8], attachments: &[Attachment]) {
    let f = satchel_mime::parse(full).unwrap();
    let stripped = satchel_mime::parse(&satchel_mime::strip_attachments(full).unwrap()).unwrap();
    let s = satchel_mime::parse(raw).expect("slim copy parses");
    assert!(raw.len() < 3000, "{} bytes", raw.len());
    assert_eq!((&s.subject, &s.text, &s.html, &s.calendar_parts), (&f.subject, &f.text, &f.html, &f.calendar_parts));
    assert_eq!(s.calendar_parts, stripped.calendar_parts);
    let ids = |a: &[Attachment]| a.iter().map(|a| (a.id.clone(), a.filename.clone(), a.mime_type.clone(), a.inline, a.content_id.clone())).collect::<Vec<_>>();
    assert_eq!(ids(attachments), ids(&f.attachments));
    assert_eq!(ids(&s.attachments), ids(&f.attachments));
    assert!(!satchel_mime::inline_data_urls(raw).is_empty(), "small inline image kept");
}

/// A part of [`sample`] as the server stores it.
pub(crate) struct TestPart {
    /// Header fields and the blank line after them.
    pub headers: Vec<u8>,
    /// Contents as sent (empty for a multipart).
    pub body: Vec<u8>,
    pub children: Vec<TestPart>,
}

impl TestPart {
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let h = String::from_utf8_lossy(&self.headers).into_owned();
        h.split("\r\n").find_map(|l| l.split_once(':').filter(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()))
    }
}

/// Split a message into parts, the way a server sees it.
pub(crate) fn split(raw: &[u8]) -> TestPart {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let (headers, body) = (raw[..at].to_vec(), &raw[at..]);
    let part = TestPart { headers, body: vec![], children: vec![] };
    let Some(b) = part.header("Content-Type").and_then(|c| c.split("boundary=\"").nth(1).map(|b| b.trim_end_matches('"').to_string())) else {
        return TestPart { body: body.to_vec(), ..part };
    };
    let text = String::from_utf8_lossy(body).into_owned();
    let mut pieces: Vec<&str> = text.split(&format!("--{b}")).collect();
    pieces.remove(0);
    pieces.pop();
    let children = pieces.into_iter().map(|p| split(p.trim_start_matches("\r\n").strip_suffix("\r\n").unwrap().as_bytes())).collect();
    TestPart { children, ..part }
}

impl TestPart {
    /// type/subtype, lowercase.
    pub(crate) fn mime_type(&self) -> String {
        self.header("Content-Type").map(|c| c.split(';').next().unwrap().trim().to_ascii_lowercase()).unwrap_or_else(|| "text/plain".into())
    }

    /// Header fields as (name, unfolded value).
    pub(crate) fn fields(&self) -> Vec<(String, String)> {
        let h = String::from_utf8_lossy(&self.headers).replace("\r\n ", " ").replace("\r\n\t", " ");
        h.split("\r\n").filter_map(|l| l.split_once(':')).map(|(n, v)| (n.to_string(), v.trim().to_string())).collect()
    }

    /// Contents with the transfer encoding undone.
    pub(crate) fn decoded(&self) -> Vec<u8> {
        use base64::Engine;
        match self.header("Content-Transfer-Encoding").unwrap_or_default().to_ascii_lowercase().as_str() {
            "base64" => base64::engine::general_purpose::STANDARD.decode(self.body.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect::<Vec<_>>()).unwrap(),
            "quoted-printable" => {
                let mut out = vec![];
                let mut i = 0;
                while i < self.body.len() {
                    if self.body[i] == b'=' {
                        out.push(u8::from_str_radix(std::str::from_utf8(&self.body[i + 1..i + 3]).unwrap(), 16).unwrap());
                        i += 3;
                    } else {
                        out.push(self.body[i]);
                        i += 1;
                    }
                }
                out
            }
            _ => self.body.clone(),
        }
    }
}
