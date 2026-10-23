//! Reads a finished Word package back for verification before it is written: the package opens,
//! every part parses, every picture and link reference resolves, every picture decodes, and the
//! text it carries can be compared with what the PDF holds. Bounded for hostile or huge input.

use pw_core::{ErrorCategory, PwError, PwResult};
use quick_xml::events::Event;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use std::collections::{HashMap, HashSet};
use std::io::Read;

const MAX_ENTRIES: usize = 100_000;
const MAX_PART_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct DocxCheck {
    /// Characters of all text runs (whitespace excluded).
    pub chars: HashMap<char, usize>,
    /// The document's text, paragraphs separated by newlines (for tests and diagnostics).
    pub text: String,
    pub sections: usize,
    pub pictures: usize,
    pub links: usize,
    pub paragraphs: usize,
    pub headings: usize,
    pub tables: usize,
}

fn bad(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The Word file failed verification ({what}) and was not saved."),
    )
}

fn part(z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> PwResult<Vec<u8>> {
    let mut f = z.by_name(name).map_err(|_| bad(&format!("{name} is missing")))?;
    if f.size() > MAX_PART_BYTES {
        return Err(bad("a part is too large"));
    }
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v).map_err(|_| bad(&format!("{name} is unreadable")))?;
    Ok(v)
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// Relationship id → (target, external).
fn rels(xml: &[u8]) -> PwResult<HashMap<String, (String, bool)>> {
    let mut r = Reader::from_reader(xml);
    let mut out = HashMap::new();
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == "Relationship" => {
                let id = attr(&e, "Id").ok_or_else(|| bad("a relationship has no id"))?;
                let target = attr(&e, "Target").ok_or_else(|| bad("a relationship has no target"))?;
                let external = attr(&e, "TargetMode").as_deref() == Some("External");
                out.insert(id, (target, external));
            }
            Ok(Event::DocType(_)) => return Err(bad("unexpected document type declaration")),
            Ok(Event::Eof) => break,
            Err(_) => return Err(bad("relationships do not parse")),
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn well_formed(xml: &[u8], what: &str) -> PwResult<()> {
    let mut r = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut depth = 0i64;
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::DocType(_)) => return Err(bad("unexpected document type declaration")),
            Ok(Event::Eof) => break,
            Err(_) => return Err(bad(&format!("{what} does not parse"))),
            _ => {}
        }
        buf.clear();
    }
    if depth != 0 {
        return Err(bad(&format!("{what} is incomplete")));
    }
    Ok(())
}

pub fn check_docx(bytes: &[u8]) -> PwResult<DocxCheck> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| bad("the package does not open"))?;
    if z.len() > MAX_ENTRIES {
        return Err(bad("too many parts"));
    }
    for name in [
        "[Content_Types].xml",
        "_rels/.rels",
        "word/styles.xml",
        "word/settings.xml",
        "docProps/core.xml",
        "docProps/app.xml",
    ] {
        well_formed(&part(&mut z, name)?, name)?;
    }
    let pkg = rels(&part(&mut z, "_rels/.rels")?)?;
    if !pkg.values().any(|(t, _)| t == "word/document.xml") {
        return Err(bad("the package does not point at its document"));
    }
    let doc_rels = rels(&part(&mut z, "word/_rels/document.xml.rels")?)?;
    let doc = part(&mut z, "word/document.xml")?;

    let mut out = DocxCheck::default();
    let mut r = Reader::from_reader(doc.as_slice());
    let mut buf = Vec::new();
    let mut in_text = false;
    let mut depth = 0i64;
    let mut embeds: HashSet<String> = HashSet::new();
    loop {
        let ev = r.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(e)) => {
                depth += 1;
                match e.local_name().as_ref() {
                    "t" => in_text = true,
                    "sectPr" => out.sections += 1,
                    "p" => out.paragraphs += 1,
                    "tbl" => out.tables += 1,
                    "hyperlink" => {
                        let id = attr(&e, "r:id").ok_or_else(|| bad("a link has no target"))?;
                        match doc_rels.get(&id) {
                            Some((t, true))
                                if t.starts_with("http://")
                                    || t.starts_with("https://")
                                    || t.starts_with("mailto:") =>
                            {
                                out.links += 1
                            }
                            _ => return Err(bad("a link points nowhere or at a disallowed place")),
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                "sectPr" => out.sections += 1,
                "p" => out.paragraphs += 1,
                "blip" => {
                    let id = attr(&e, "r:embed").ok_or_else(|| bad("a picture has no data"))?;
                    out.pictures += 1;
                    embeds.insert(id);
                }
                "pStyle" if attr(&e, "w:val").is_some_and(|v| v.starts_with("Heading")) => out.headings += 1,
                "tab" => out.text.push('\t'),
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => {
                let s = t.xml10_content();
                for c in s.chars() {
                    if !c.is_whitespace() {
                        *out.chars.entry(c).or_default() += 1;
                    }
                }
                out.text.push_str(&s);
            }
            Ok(Event::GeneralRef(e)) if in_text => {
                let c = match e.resolve_char_ref() {
                    Ok(Some(c)) => c,
                    Ok(None) => match e.xml10_content().as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(bad("unknown entity in text")),
                    },
                    Err(_) => return Err(bad("invalid character reference")),
                };
                *out.chars.entry(c).or_default() += 1;
                out.text.push(c);
            }
            Ok(Event::End(e)) => {
                depth -= 1;
                match e.local_name().as_ref() {
                    "t" => in_text = false,
                    "p" => out.text.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::DocType(_)) => return Err(bad("unexpected document type declaration")),
            Ok(Event::Eof) => break,
            Err(_) => return Err(bad("the document does not parse")),
            _ => {}
        }
        buf.clear();
    }
    if depth != 0 {
        return Err(bad("the document is incomplete"));
    }
    for id in embeds {
        let Some((target, false)) = doc_rels.get(&id) else { return Err(bad("a picture points nowhere")) };
        let data = part(&mut z, &format!("word/{target}"))?;
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&data))
            .with_guessed_format()
            .map_err(|_| bad("a picture does not decode"))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(1024 * 1024 * 1024);
        reader.limits(limits);
        reader.decode().map_err(|_| bad("a picture does not decode"))?;
    }
    Ok(out)
}
