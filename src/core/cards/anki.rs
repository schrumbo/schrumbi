//! Reads Anki packages (`.apkg`, `.colpkg`) and text exports, and writes them as notes with cards.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow};
use regex::Regex;
use serde_json::Value;

use super::fsrs::Rating;
use super::parse;
use super::store::{CardState, Phase, ReviewLog};

/// How far Anki had got with one card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Progress {
    pub phase: Phase,
    pub stability: f64,
    pub difficulty: f64,
    pub due: i64,
    pub last_review: Option<i64>,
    pub reps: u32,
    pub lapses: u32,
    pub suspended: bool,
    pub anki_id: i64,
}

/// One note of a package as text, with the progress of its cards in the order the text makes them.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportNote {
    pub text: String,
    pub progress: Vec<Progress>,
    pub tags: Vec<String>,
}

/// What a package holds.
#[derive(Debug, Default)]
pub struct Package {
    /// Deck name with its parts, and its notes.
    pub decks: BTreeMap<Vec<String>, Vec<ImportNote>>,
    /// Media files by name.
    pub media: Vec<(String, Vec<u8>)>,
    pub reviews: Vec<(i64, i64, u8, u8, f64)>,
    pub cards: usize,
    pub with_progress: usize,
    /// Media files left out because they were too large.
    pub skipped_media: usize,
}

/// What an import did.
#[derive(Debug, Default)]
pub struct ImportResult {
    pub added: usize,
    pub duplicates: usize,
    pub created: Vec<PathBuf>,
    pub media: usize,
    pub states: Vec<(String, CardState)>,
    pub log: Vec<ReviewLog>,
    /// Notes that were there before and got cards added at the end, with their bytes before.
    pub appended: Vec<(PathBuf, Vec<u8>)>,
    /// Media files left out because their name cannot be a file name.
    pub skipped_media: usize,
}

/// Reads a package or a text export, whichever the file is.
pub fn read(path: &Path) -> Result<Package> {
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "apkg" | "colpkg" => read_package(path),
        "txt" | "tsv" | "csv" => read_text(path),
        _ => Err(anyhow!("Diese Datei ist kein Anki-Export.")),
    }
}

const COLLECTION_LIMIT: u64 = 512 << 20;
const MEDIA_LIMIT: u64 = 64 << 20;
const TOTAL_LIMIT: u64 = 2 << 30;

fn too_large() -> anyhow::Error {
    anyhow!(crate::core::i18n::tr("Dieses Anki-Paket ist zu groß zum Importieren. Exportiere die Stapel in Anki einzeln."))
}

/// Everything `from` holds, unless that is more than `limit` bytes.
fn read_capped(from: impl Read, limit: u64) -> Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    from.take(limit + 1).read_to_end(&mut out)?;
    Ok((out.len() as u64 <= limit).then_some(out))
}

fn zstd(bytes: &[u8], limit: u64) -> Result<Option<Vec<u8>>> {
    read_capped(ruzstd::decoding::StreamingDecoder::new(bytes).map_err(|e| anyhow!("{e}"))?, limit)
}

/// The next value of a protobuf message: field number, and its bytes when length-delimited.
fn proto_fields(mut data: &[u8]) -> Vec<(u32, Vec<u8>, u64)> {
    fn varint(data: &mut &[u8]) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (byte, rest) = data.split_first()?;
            *data = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    let mut out = Vec::new();
    while let Some(key) = varint(&mut data) {
        let (field, kind) = ((key >> 3) as u32, key & 7);
        match kind {
            0 => {
                let Some(v) = varint(&mut data) else { break };
                out.push((field, Vec::new(), v));
            }
            1 if data.len() >= 8 => data = &data[8..],
            5 if data.len() >= 4 => data = &data[4..],
            2 => {
                let Some(len) = varint(&mut data) else { break };
                let len = len as usize;
                if data.len() < len {
                    break;
                }
                out.push((field, data[..len].to_vec(), 0));
                data = &data[len..];
            }
            _ => break,
        }
    }
    out
}

fn proto_text(message: &[u8], field: u32) -> String {
    proto_fields(message).into_iter().find(|(f, ..)| *f == field).map(|(_, bytes, _)| String::from_utf8_lossy(&bytes).to_string()).unwrap_or_default()
}

fn proto_number(message: &[u8], field: u32) -> u64 {
    proto_fields(message).into_iter().find(|(f, ..)| *f == field).map_or(0, |(_, _, v)| v)
}

#[derive(Debug, Clone, Default)]
struct NoteType {
    cloze: bool,
    fields: Vec<String>,
    templates: Vec<(String, String)>,
}

fn read_package(path: &Path) -> Result<Package> {
    let file = std::fs::File::open(path).with_context(|| path.display().to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| anyhow!("Die Datei ist kein gültiges Anki-Paket."))?;
    let names: Vec<String> = zip.file_names().flatten().map(|n| n.to_string()).collect();
    let read_entry = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str, limit: u64| -> Result<Option<Vec<u8>>> { read_capped(zip.by_name(name)?, limit) };
    let collection = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str| read_entry(zip, name, COLLECTION_LIMIT)?.ok_or_else(too_large);
    let (database, modern) = if names.iter().any(|n| n == "collection.anki21b") {
        (zstd(&collection(&mut zip, "collection.anki21b")?, COLLECTION_LIMIT)?.ok_or_else(too_large)?, true)
    } else if names.iter().any(|n| n == "collection.anki21") {
        (collection(&mut zip, "collection.anki21")?, false)
    } else if names.iter().any(|n| n == "collection.anki2") {
        (collection(&mut zip, "collection.anki2")?, false)
    } else {
        return Err(anyhow!("In diesem Paket steht keine Sammlung."));
    };
    let mut media = Vec::new();
    let (mut total, mut skipped) = (database.len() as u64, 0);
    if let Ok(Some(map)) = read_entry(&mut zip, "media", COLLECTION_LIMIT) {
        let named: Vec<(String, String)> = match serde_json::from_slice::<HashMap<String, String>>(&map) {
            Ok(old) => old.into_iter().collect(),
            Err(_) => {
                let plain = zstd(&map, COLLECTION_LIMIT).ok().flatten().unwrap_or(map);
                proto_fields(&plain).into_iter().filter(|(f, ..)| *f == 1).enumerate().map(|(i, (_, entry, _))| (i.to_string(), proto_text(&entry, 1))).collect()
            }
        };
        for (index, name) in named {
            let Ok(read) = read_entry(&mut zip, &index, MEDIA_LIMIT) else { continue };
            let read = match (read, modern) {
                (Some(bytes), true) => zstd(&bytes, MEDIA_LIMIT).unwrap_or(Some(bytes)),
                (read, _) => read,
            };
            let Some(bytes) = read else {
                skipped += 1;
                continue;
            };
            total += bytes.len() as u64;
            if total > TOTAL_LIMIT {
                return Err(too_large());
            }
            if !name.contains(['/', '\\']) && !name.starts_with('.') {
                media.push((name, bytes));
            }
        }
    }
    let tmp = std::env::temp_dir().join(format!("schrumbi-anki-{}-{}.db", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
    std::fs::write(&tmp, database)?;
    let read = read_database(&tmp, media);
    let _ = std::fs::remove_file(&tmp);
    read.map(|package| Package { skipped_media: skipped, ..package })
}

fn read_database(file: &Path, media: Vec<(String, Vec<u8>)>) -> Result<Package> {
    let db = rusqlite::Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_table = |name: &str| db.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1", [name], |_| Ok(())).is_ok();
    let crt: i64 = db.query_row("SELECT crt FROM col", [], |r| r.get(0)).unwrap_or(0);
    let mut types: HashMap<i64, NoteType> = HashMap::new();
    let mut decks: HashMap<i64, Vec<String>> = HashMap::new();
    if has_table("notetypes") {
        let mut query = db.prepare("SELECT id, config FROM notetypes")?;
        let rows = query.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        for (id, config) in rows.flatten() {
            types.insert(id, NoteType { cloze: proto_number(&config, 1) == 1, ..Default::default() });
        }
        let mut query = db.prepare("SELECT ntid, ord, name FROM fields")?;
        let mut fields: Vec<(i64, i64, String)> = query.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.flatten().collect();
        fields.sort_by_key(|f| (f.0, f.1));
        for (id, _, name) in fields {
            types.entry(id).or_default().fields.push(name);
        }
        let mut query = db.prepare("SELECT ntid, ord, config FROM templates")?;
        let mut templates: Vec<(i64, i64, Vec<u8>)> = query.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.flatten().collect();
        templates.sort_by_key(|t| (t.0, t.1));
        for (id, _, config) in templates {
            types.entry(id).or_default().templates.push((proto_text(&config, 1), proto_text(&config, 2)));
        }
        let mut query = db.prepare("SELECT id, name FROM decks")?;
        for (id, name) in query.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?.flatten() {
            decks.insert(id, name.split('\u{1f}').map(str::to_string).collect());
        }
    } else {
        let (models, deck_list): (String, String) = db.query_row("SELECT models, decks FROM col", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let models: Value = serde_json::from_str(&models)?;
        for (id, model) in models.as_object().into_iter().flatten() {
            let mut fields: Vec<(i64, String)> = model["flds"].as_array().into_iter().flatten().map(|f| (f["ord"].as_i64().unwrap_or(0), f["name"].as_str().unwrap_or_default().to_string())).collect();
            fields.sort();
            let mut templates: Vec<(i64, String, String)> = model["tmpls"].as_array().into_iter().flatten().map(|t| (t["ord"].as_i64().unwrap_or(0), t["qfmt"].as_str().unwrap_or_default().to_string(), t["afmt"].as_str().unwrap_or_default().to_string())).collect();
            templates.sort();
            types.insert(id.parse().unwrap_or(0), NoteType { cloze: model["type"].as_i64() == Some(1), fields: fields.into_iter().map(|f| f.1).collect(), templates: templates.into_iter().map(|t| (t.1, t.2)).collect() });
        }
        let deck_list: Value = serde_json::from_str(&deck_list)?;
        for (id, deck) in deck_list.as_object().into_iter().flatten() {
            decks.insert(id.parse().unwrap_or(0), deck["name"].as_str().unwrap_or_default().split("::").map(str::to_string).collect());
        }
    }
    let mut query = db.prepare("SELECT id, nid, did, ord, type, queue, due, ivl, factor, reps, lapses, data, odue, odid FROM cards")?;
    struct Row {
        deck: i64,
        ord: i64,
        progress: Progress,
    }
    let mut by_note: HashMap<i64, Vec<Row>> = HashMap::new();
    let rows = query.query_map([], |r| {
        let (id, nid, mut did, ord): (i64, i64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        let (kind, queue, mut due, ivl, factor, reps, lapses): (i64, i64, i64, i64, i64, i64, i64) = (r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?);
        let data: String = r.get(11).unwrap_or_default();
        let (odue, odid): (i64, i64) = (r.get(12).unwrap_or(0), r.get(13).unwrap_or(0));
        if odid != 0 {
            did = odid;
            due = odue;
        }
        Ok((nid, Row { deck: did, ord, progress: progress_of(kind, queue, due, ivl, factor, reps, lapses, &data, crt, id) }))
    })?;
    for (nid, row) in rows.flatten() {
        by_note.entry(nid).or_default().push(row);
    }
    let mut out = Package { media, ..Default::default() };
    let mut query = db.prepare("SELECT id, mid, flds, tags FROM notes")?;
    let notes: Vec<(i64, i64, String, String)> = query.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.flatten().collect();
    for (nid, mid, flds, tags) in notes {
        let Some(kind) = types.get(&mid) else { continue };
        let Some(mut cards) = by_note.remove(&nid) else { continue };
        cards.sort_by_key(|c| c.ord);
        let values: HashMap<&str, &str> = kind.fields.iter().map(String::as_str).zip(flds.split('\u{1f}')).collect();
        let deck = decks.get(&cards[0].deck).cloned().unwrap_or_else(|| vec!["Anki".to_string()]);
        let tags: Vec<String> = tags.split_whitespace().map(|t| t.replace("::", "/")).collect();
        let progress: Vec<Progress> = cards.iter().map(|c| c.progress.clone()).collect();
        out.with_progress += progress.iter().filter(|p| p.phase != Phase::New || p.suspended).count();
        out.cards += cards.len();
        let notes = render_note(kind, &values, &cards.iter().map(|c| c.ord as usize).collect::<Vec<_>>(), &progress, &tags);
        out.decks.entry(deck).or_default().extend(notes);
    }
    if has_table("revlog") {
        let mut query = db.prepare("SELECT id, cid, ease, type, ivl FROM revlog")?;
        out.reviews = query.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)? as u8, r.get::<_, i64>(3)? as u8, r.get::<_, i64>(4)? as f64)))?.flatten().collect();
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn progress_of(kind: i64, queue: i64, due: i64, ivl: i64, factor: i64, reps: i64, lapses: i64, data: &str, crt: i64, id: i64) -> Progress {
    let memory: Value = serde_json::from_str(data).unwrap_or(Value::Null);
    let stability = memory["s"].as_f64().unwrap_or((ivl.max(1)) as f64);
    let difficulty = memory["d"].as_f64().unwrap_or_else(|| (10. - (factor.max(1300) - 1300) as f64 / 1700. * 9.).clamp(1., 10.));
    let phase = match kind {
        1 => Phase::Learning,
        2 => Phase::Review,
        3 => Phase::Relearning,
        _ => Phase::New,
    };
    const CENTURY: i64 = 36_500 * 86_400;
    let due_at = if phase == Phase::Review || due < 1_000_000_000 { crt.saturating_add(due.clamp(0, 365_000).saturating_mul(86_400)) } else { due };
    if phase != Phase::New && (due_at > crt.max(0).saturating_add(2 * CENTURY) || reps <= 0) {
        return Progress { suspended: queue == -1, anki_id: id, ..Default::default() };
    }
    let last_review = (phase != Phase::New).then_some(due_at.saturating_sub(ivl.clamp(0, 365_000) * 86_400));
    Progress { phase, stability, difficulty, due: due_at, last_review, reps: reps.clamp(0, u32::MAX as i64) as u32, lapses: lapses.clamp(0, u32::MAX as i64) as u32, suspended: queue == -1, anki_id: id }
}

fn render_note(kind: &NoteType, fields: &HashMap<&str, &str>, ords: &[usize], progress: &[Progress], tags: &[String]) -> Vec<ImportNote> {
    if kind.cloze {
        let template = kind.templates.first().map_or("", |t| t.0.as_str());
        let cloze_field = CLOZE_FIELD.captures(template).map_or(String::new(), |c| c[1].trim().to_string());
        let source = fields.get(cloze_field.as_str()).copied().or_else(|| fields.values().next().copied()).unwrap_or_default();
        let body = one_line(&to_markdown(source));
        let extra = kind.templates.first().map_or(String::new(), |t| to_markdown(&fill(&t.1, fields, Some(&cloze_field))));
        let extra: String = extra.lines().filter(|l| !l.trim().is_empty() && !l.contains("{{c")).collect::<Vec<_>>().join("\n");
        let extra = extra.lines().filter(|l| one_line(l) != body).collect::<Vec<_>>().join("\n");
        let text = if extra.trim().is_empty() { body } else { format!("{body}\n?\n{extra}") };
        return vec![ImportNote { text, progress: progress.to_vec(), tags: tags.to_vec() }];
    }
    let mut sides: Vec<(String, String, Progress)> = Vec::new();
    for (ord, card) in ords.iter().zip(progress) {
        let Some((question, answer)) = kind.templates.get(*ord) else { continue };
        let front = one_line(&to_markdown(&fill(question, fields, None)));
        let answer = FRONT_SIDE.replace_all(answer, "");
        let back = to_markdown(&fill(&answer, fields, None));
        let back = back.strip_prefix(front.as_str()).map(|b| b.trim_start().to_string()).unwrap_or(back);
        sides.push((front, back, card.clone()));
    }
    if sides.len() == 2 && sides[0].0 == one_line(&sides[1].1) && one_line(&sides[0].1) == sides[1].0 {
        let (a, b) = (sides[0].clone(), sides[1].clone());
        return vec![ImportNote { text: parse::write_card(&a.0, &a.1, true), progress: vec![a.2, b.2], tags: tags.to_vec() }];
    }
    sides.into_iter().filter(|(front, back, _)| !front.is_empty() && !back.trim().is_empty()).map(|(front, back, card)| ImportNote { text: parse::write_card(&front, &back, false), progress: vec![card], tags: tags.to_vec() }).collect()
}

static CLOZE_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{(?:[^}:]+:)*cloze:([^}]+)\}\}").unwrap());
static FRONT_SIDE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\{\{FrontSide\}\}|(?i)<hr[^>]*id=["']?answer["']?[^>]*>"#).unwrap());

/// A card template with the note's fields put in; `leave` keeps that field out, it is shown elsewhere.
fn fill(template: &str, fields: &HashMap<&str, &str>, leave: Option<&str>) -> String {
    static SECTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{([#^])([^}]+)\}\}").unwrap());
    static FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{([^}]+)\}\}").unwrap());
    let mut text = template.to_string();
    let value_of = |name: &str| fields.get(name.trim()).copied().unwrap_or_default().to_string();
    while let Some(c) = SECTION.captures(&text) {
        let whole = c.get(0).unwrap().range();
        let name = c[2].to_string();
        let shown = !value_of(&name).trim().is_empty();
        let wanted = if &c[1] == "#" { shown } else { !shown };
        let close = format!("{{{{/{name}}}}}");
        let Some(end) = text[whole.end..].find(&close).map(|e| whole.end + e) else {
            text.replace_range(whole, "");
            continue;
        };
        let inner = text[whole.end..end].to_string();
        text.replace_range(whole.start..end + close.len(), if wanted { &inner } else { "" });
    }
    FIELD
        .replace_all(&text, |c: &regex::Captures| {
            let spec = &c[1];
            let name = spec.rsplit(':').next().unwrap_or(spec).trim();
            if spec.starts_with("type:") || leave.is_some_and(|l| l == name) || matches!(spec, "Tags" | "Deck" | "Subdeck" | "Type" | "Card" | "CardFlag") {
                return String::new();
            }
            value_of(name)
        })
        .into_owned()
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn entities(text: &str) -> String {
    static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#(x?[0-9a-fA-F]+);").unwrap());
    let text = text.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    let text = NUMBER.replace_all(&text, |c: &regex::Captures| {
        let code = &c[1];
        let parsed = code.strip_prefix('x').map_or_else(|| code.parse().ok(), |h| u32::from_str_radix(h, 16).ok());
        parsed.and_then(char::from_u32).map_or(String::new(), |ch| ch.to_string())
    });
    text.replace("&amp;", "&")
}

/// A field's HTML as Markdown, with formulas and media in the forms notes use.
pub fn to_markdown(html: &str) -> String {
    static DROP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(style|script)\b.*?</\s*(style|script)\s*>|<!--.*?-->").unwrap());
    static SOUND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[sound:([^\]]+)\]").unwrap());
    static IMAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<img\b[^>]*?src=["']?([^"'>\s]+)["']?[^>]*>"#).unwrap());
    static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<a\b[^>]*?href=["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap());
    static MATHJAX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<anki-mathjax(\s+block="true")?[^>]*>(.*?)</anki-mathjax>"#).unwrap());
    static LATEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)\[\$\$\](.*?)\[/\$\$\]|\\\[(.*?)\\\]|\[latex\](.*?)\[/latex\]").unwrap());
    static INLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)\[\$\](.*?)\[/\$\]|\\\((.*?)\\\)").unwrap());
    static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<h([1-6])[^>]*>(.*?)</h[1-6]>").unwrap());
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)</?[a-zA-Z][^>]*>").unwrap());
    static SIMPLE: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
        [
        (r"(?i)<br\s*/?>", "\n"),
        (r"(?i)</(div|p|li|tr|ul|ol|h[1-6]|blockquote|pre)>", "\n"),
        (r"(?i)<li[^>]*>", "- "),
        (r"(?i)</?(b|strong)(\s[^>]*)?>", "**"),
        (r"(?i)</?(i|em)(\s[^>]*)?>", "*"),
        (r"(?i)</?(s|strike|del)(\s[^>]*)?>", "~~"),
        (r"(?i)</?mark(\s[^>]*)?>", "=="),
        (r"(?i)</?code(\s[^>]*)?>", "`"),
        (r"(?i)<u(\s[^>]*)?>", "\u{e001}"),
        (r"(?i)</u>", "\u{e002}"),
        (r"(?i)<sup(\s[^>]*)?>", "\u{e003}"),
        (r"(?i)</sup>", "\u{e004}"),
        (r"(?i)<sub(\s[^>]*)?>", "\u{e005}"),
        (r"(?i)</sub>", "\u{e006}"),
        ]
        .into_iter()
        .map(|(pattern, to)| (Regex::new(pattern).unwrap(), to))
        .collect()
    });
    let text = DROP.replace_all(html, "");
    let text = SOUND.replace_all(&text, "![[$1]]");
    let text = MATHJAX.replace_all(&text, |c: &regex::Captures| if c.get(1).is_some() { format!("$${}$$", &c[2]) } else { format!("${}$", &c[2]) });
    let text = LATEX.replace_all(&text, |c: &regex::Captures| format!("$${}$$", c.get(1).or(c.get(2)).or(c.get(3)).map_or("", |m| m.as_str())));
    let text = INLINE.replace_all(&text, |c: &regex::Captures| format!("${}$", c.get(1).or(c.get(2)).map_or("", |m| m.as_str())));
    let text = IMAGE.replace_all(&text, |c: &regex::Captures| {
        let src = entities(&c[1]);
        if src.contains("://") { format!("![]({src})") } else { format!("![[{}]]", crate::core::markdown::percent_decode(&src)) }
    });
    let text = LINK.replace_all(&text, "[$2]($1)");
    let text = HEADING.replace_all(&text, |c: &regex::Captures| format!("{} {}\n", "#".repeat(c[1].parse().unwrap_or(1)), &c[2]));
    let mut text = text.into_owned();
    for (pattern, to) in SIMPLE.iter() {
        if let std::borrow::Cow::Owned(replaced) = pattern.replace_all(&text, *to) {
            text = replaced;
        }
    }
    let text = TAG.replace_all(&text, "");
    let text = entities(&text);
    let text = text.replace('\u{e001}', "<u>").replace('\u{e002}', "</u>").replace('\u{e003}', "<sup>").replace('\u{e004}', "</sup>").replace('\u{e005}', "<sub>").replace('\u{e006}', "</sub>");
    text.lines().map(str::trim_end).map(|l| if l.trim().is_empty() { "" } else { l }).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n").trim().to_string()
}

fn read_text(path: &Path) -> Result<Package> {
    let bytes = std::fs::read(path)?;
    let content = String::from_utf8_lossy(&bytes);
    let mut separator = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) { ',' } else { '\t' };
    let mut html = false;
    let mut rows = Vec::new();
    for line in content.lines() {
        if let Some(setting) = line.strip_prefix('#') {
            if let Some((key, value)) = setting.split_once(':') {
                match (key, value.trim()) {
                    ("separator", "tab" | "Tab") => separator = '\t',
                    ("separator", "comma" | "Comma") => separator = ',',
                    ("separator", "semicolon" | "Semicolon") => separator = ';',
                    ("separator", "pipe" | "Pipe") => separator = '|',
                    ("separator", "space" | "Space") => separator = ' ',
                    ("html", value) => html = value == "true",
                    _ => {}
                }
                continue;
            }
        }
        rows.push(line);
    }
    let name = path.file_stem().map_or_else(|| "Anki".to_string(), |n| n.to_string_lossy().to_string());
    let mut out = Package::default();
    for row in rows {
        let cells = split_row(row, separator);
        let (Some(front), Some(back)) = (cells.first(), cells.get(1)) else { continue };
        let side = |s: &str| if html { to_markdown(s) } else { s.trim().to_string() };
        let (front, back) = (one_line(&side(front)), side(back));
        if front.is_empty() || back.is_empty() {
            continue;
        }
        out.cards += 1;
        out.decks.entry(vec![name.clone()]).or_default().push(ImportNote { text: parse::write_card(&front, &back, false), progress: vec![Progress::default()], tags: Vec::new() });
    }
    if out.cards == 0 {
        return Err(anyhow!("In dieser Datei stehen keine Karten."));
    }
    Ok(out)
}

fn split_row(row: &str, separator: char) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut quoted = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cells.last_mut().unwrap().push('"');
            }
            '"' => quoted = !quoted,
            c if c == separator && !quoted => cells.push(String::new()),
            c => cells.last_mut().unwrap().push(c),
        }
    }
    cells
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']') { '-' } else { c }).collect();
    crate::core::text::safe_file_name(&cleaned, "Anki")
}

/// The note text with ` (n)` behind its question, so a card with the same question as another one gets an id of its own.
fn distinguished(text: &str, front: &str, n: usize) -> String {
    let at = text.find(front).filter(|_| !front.is_empty()).map_or_else(|| text.find('\n').unwrap_or(text.len()), |at| at + front.len());
    format!("{} ({n}){}", &text[..at], &text[at..])
}

/// Writes a package into notes under `folder`, media beside them. What is already there is not written twice.
pub fn write(package: Package, folder: &Path, keep_progress: bool, known: &HashSet<String>, rel_of: impl Fn(&Path) -> String) -> Result<ImportResult> {
    let mut result = ImportResult { skipped_media: package.skipped_media, ..Default::default() };
    let mut seen: HashSet<String> = HashSet::new();
    let mut texts: HashSet<String> = HashSet::new();
    let mut by_anki: HashMap<i64, String> = HashMap::new();
    std::fs::create_dir_all(folder)?;
    for (deck, notes) in package.decks {
        let mut path = folder.to_path_buf();
        for part in &deck[..deck.len().saturating_sub(1)] {
            path.push(safe_name(part));
        }
        path.push(format!("{}.md", safe_name(deck.last().map_or("Anki", String::as_str))));
        let before = match std::fs::read(&path) {
            Ok(bytes) if String::from_utf8(bytes.clone()).is_ok() => Some(bytes),
            Ok(_) => {
                path = crate::core::library::unique_path(&path);
                None
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(anyhow!(crate::core::i18n::trf("„%@“ ist gerade nicht lesbar, deshalb wurde nichts importiert: %@", &[&path.file_name().unwrap_or_default().to_string_lossy(), &e]))),
        };
        let mut text = String::new();
        let mut tags: Vec<String> = Vec::new();
        for note in notes {
            let mut body = note.text;
            let mut cards = parse::cards_in(&body);
            if cards.is_empty() {
                continue;
            }
            if cards.iter().all(|c| known.contains(&c.id)) || !texts.insert(body.clone()) {
                result.duplicates += cards.len();
                continue;
            }
            let front = cards[0].front.clone();
            let mut n = 2;
            while cards.iter().any(|c| seen.contains(&c.id) || known.contains(&c.id)) && n < 1000 {
                let next = distinguished(&body, &front, n);
                cards = parse::cards_in(&next);
                if cards.is_empty() {
                    cards = parse::cards_in(&body);
                    break;
                }
                body = next;
                n += 1;
            }
            for (card, progress) in cards.iter().zip(&note.progress) {
                seen.insert(card.id.clone());
                by_anki.insert(progress.anki_id, card.id.clone());
                result.added += 1;
                if keep_progress && (progress.phase != Phase::New || progress.suspended) {
                    result.states.push((card.id.clone(), CardState {
                        phase: progress.phase,
                        stability: progress.stability,
                        difficulty: progress.difficulty,
                        due: progress.due,
                        last_review: progress.last_review,
                        step: 0,
                        reps: progress.reps,
                        lapses: progress.lapses,
                        suspended: progress.suspended,
                        buried_until: None,
                        note: rel_of(&path),
                        question: card.front.clone(),
                    }));
                }
            }
            for tag in note.tags {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
            text.push_str(&body);
            text.push_str("\n\n");
        }
        if text.is_empty() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existing = before.as_ref().map(|b| String::from_utf8_lossy(b).into_owned());
        let start = match &existing {
            Some(old) => old.clone(),
            None if !tags.is_empty() => format!("---\ntags: [{}]\n---\n\n", tags.join(", ")),
            None => String::new(),
        };
        let joined = if start.is_empty() || start.ends_with("\n\n") { start } else if start.ends_with('\n') { start + "\n" } else { start + "\n\n" };
        crate::core::note_buffer::write_atomic(&path, (joined + &text).as_bytes())?;
        match before {
            Some(bytes) => result.appended.push((path, bytes)),
            None => result.created.push(path),
        }
    }
    if !package.media.is_empty() {
        let media = folder.join("Medien");
        std::fs::create_dir_all(&media)?;
        for (name, bytes) in package.media {
            if crate::core::text::file_name_problem(&name).is_some() || name.starts_with('.') {
                result.skipped_media += 1;
                continue;
            }
            let target = media.join(&name);
            if !target.exists() {
                std::fs::write(&target, bytes)?;
                result.created.push(target);
            }
            result.media += 1;
        }
    }
    if keep_progress {
        for (at, card, ease, kind, ivl) in package.reviews {
            let Some(id) = by_anki.get(&card) else { continue };
            let rating = match ease {
                1 => Rating::Again,
                2 => Rating::Hard,
                3 => Rating::Good,
                4 => Rating::Easy,
                _ => continue,
            };
            let phase = match kind {
                0 => Phase::Learning,
                1 => Phase::Review,
                2 => Phase::Relearning,
                _ => continue,
            };
            result.log.push(ReviewLog { card: id.clone(), at: at / 1000, rating, phase, days: ivl.max(0.) });
        }
        result.log.sort_by_key(|l| l.at);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn html_becomes_markdown() {
        assert_eq!(to_markdown("a<br>b"), "a\nb");
        assert_eq!(to_markdown("<div>x</div><div>y</div>"), "x\ny");
        assert_eq!(to_markdown("<b>f</b> <i>k</i> &amp; &nbsp;z"), "**f** *k* &  z");
        assert_eq!(to_markdown(r#"<img src="a%20b.png" width=3>"#), "![[a b.png]]");
        assert_eq!(to_markdown("[sound:t.mp3]"), "![[t.mp3]]");
        assert_eq!(to_markdown(r"\(x^2\) und \[y\] und [$]z[/$] [latex]w[/latex]"), "$x^2$ und $$y$$ und $z$ $$w$$");
        assert_eq!(to_markdown(r#"<anki-mathjax block="true">a</anki-mathjax>"#), "$$a$$");
        assert_eq!(to_markdown(r#"<a href="https://x.y">t</a>"#), "[t](https://x.y)");
        assert_eq!(to_markdown("<ul><li>a</li><li>b</li></ul>"), "- a\n- b");
        assert_eq!(to_markdown("<style>.a{}</style>t"), "t");
    }

    #[test]
    fn templates_fill_in() {
        let fields: HashMap<&str, &str> = [("Front", "F"), ("Back", ""), ("Extra", "E")].into_iter().collect();
        assert_eq!(fill("{{Front}}{{#Back}}X{{/Back}}{{^Back}}Y{{/Back}}{{text:Extra}}{{type:Front}}", &fields, None), "FYE");
    }

    fn sample_database(file: &Path) {
        let db = rusqlite::Connection::open(file).unwrap();
        db.execute_batch(
            r#"CREATE TABLE col (crt INT, models TEXT, decks TEXT);
            CREATE TABLE notes (id INT, mid INT, flds TEXT, tags TEXT);
            CREATE TABLE cards (id INT, nid INT, did INT, ord INT, type INT, queue INT, due INT, ivl INT, factor INT, reps INT, lapses INT, data TEXT, odue INT, odid INT);
            CREATE TABLE revlog (id INT, cid INT, ease INT, type INT, ivl INT);"#,
        )
        .unwrap();
        let models = serde_json::json!({
            "1": {"type": 0, "flds": [{"name": "Front", "ord": 0}, {"name": "Back", "ord": 1}], "tmpls": [{"ord": 0, "qfmt": "{{Front}}", "afmt": "{{FrontSide}}<hr id=answer>{{Back}}"}, {"ord": 1, "qfmt": "{{Back}}", "afmt": "{{FrontSide}}<hr id=answer>{{Front}}"}]},
            "2": {"type": 1, "flds": [{"name": "Text", "ord": 0}, {"name": "Extra", "ord": 1}], "tmpls": [{"ord": 0, "qfmt": "{{cloze:Text}}", "afmt": "{{cloze:Text}}<br>{{Extra}}"}]}
        });
        let decks = serde_json::json!({"10": {"name": "Mathe::Analysis"}});
        db.execute("INSERT INTO col VALUES (1700000000, ?1, ?2)", [models.to_string(), decks.to_string()]).unwrap();
        db.execute("INSERT INTO notes VALUES (1, 1, 'Was ist <b>2+2</b>?\u{1f}Vier <img src=\"a.png\">', 'tag1 x::y')", []).unwrap();
        db.execute("INSERT INTO notes VALUES (2, 2, 'Der {{c1::Satz}} von {{c2::Euler}}\u{1f}Mehr dazu', '')", []).unwrap();
        db.execute("INSERT INTO cards VALUES (100, 1, 10, 0, 2, 2, 100, 30, 2500, 9, 1, '{\"s\":42.5,\"d\":6.5}', 0, 0)", []).unwrap();
        db.execute("INSERT INTO cards VALUES (101, 1, 10, 1, 0, -1, 0, 0, 0, 0, 0, '', 0, 0)", []).unwrap();
        db.execute("INSERT INTO cards VALUES (200, 2, 10, 0, 0, 0, 5, 0, 0, 0, 0, '', 0, 0)", []).unwrap();
        db.execute("INSERT INTO cards VALUES (201, 2, 10, 1, 0, 0, 6, 0, 0, 0, 0, '', 0, 0)", []).unwrap();
        db.execute("INSERT INTO revlog VALUES (1700000000000, 100, 3, 1, 30)", []).unwrap();
    }

    fn sample_package(dir: &Path, modern: bool) -> PathBuf {
        let db_path = dir.join("c.db");
        sample_database(&db_path);
        let database = std::fs::read(&db_path).unwrap();
        let path = dir.join(if modern { "m.apkg" } else { "o.apkg" });
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let image = b"png bytes".to_vec();
        if modern {
            zip.start_file("collection.anki21b", options).unwrap();
            zip.write_all(&ruzstd::encoding::compress_to_vec(&database[..], ruzstd::encoding::CompressionLevel::Fastest)).unwrap();
            let mut entry = vec![0x0a, 5];
            entry.extend_from_slice(b"a.png");
            let mut map = vec![0x0a, entry.len() as u8];
            map.extend(entry);
            zip.start_file("media", options).unwrap();
            zip.write_all(&ruzstd::encoding::compress_to_vec(&map[..], ruzstd::encoding::CompressionLevel::Fastest)).unwrap();
            zip.start_file("0", options).unwrap();
            zip.write_all(&ruzstd::encoding::compress_to_vec(&image[..], ruzstd::encoding::CompressionLevel::Fastest)).unwrap();
        } else {
            zip.start_file("collection.anki21", options).unwrap();
            zip.write_all(&database).unwrap();
            zip.start_file("media", options).unwrap();
            zip.write_all(br#"{"0":"a.png"}"#).unwrap();
            zip.start_file("0", options).unwrap();
            zip.write_all(&image).unwrap();
        }
        zip.finish().unwrap();
        std::fs::remove_file(db_path).unwrap();
        path
    }

    fn check(modern: bool) {
        let dir = std::env::temp_dir().join(format!("schrumbi-anki-test-{}-{modern}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let package = read(&sample_package(&dir, modern)).unwrap();
        assert_eq!(package.cards, 4);
        assert_eq!(package.media.len(), 1);
        let target = dir.join("Anki");
        let result = write(package, &target, true, &HashSet::new(), |p| p.to_string_lossy().to_string()).unwrap();
        let note = std::fs::read_to_string(target.join("Mathe/Analysis.md")).unwrap();
        assert!(note.starts_with("---\ntags: [tag1, x/y]\n---\n"), "{note}");
        assert!(note.contains("Was ist **2+2**?"), "{note}");
        assert!(note.contains("{{c1::Satz}} von {{c2::Euler}}"), "{note}");
        assert!(note.contains("Mehr dazu"), "{note}");
        assert_eq!(std::fs::read(target.join("Medien/a.png")).unwrap(), b"png bytes");
        let cards = parse::cards_in(&note);
        assert_eq!(cards.len(), 4, "{note}");
        assert_eq!(result.added, 4);
        let known = result.states.iter().find(|(_, s)| s.phase == Phase::Review).unwrap();
        assert_eq!(known.1.stability, 42.5);
        assert_eq!(known.1.due, 1700000000 + 100 * 86_400);
        assert!(result.states.iter().any(|(_, s)| s.suspended));
        assert_eq!(result.log.len(), 1);
        assert_eq!(result.states.len(), 2, "only the learned and the suspended card bring a state");
        assert!(result.states.iter().all(|(_, s)| s.phase == Phase::New || s.reps > 0), "a card never learned is never scheduled as learned");
        assert!(cards.iter().filter(|c| c.kind == parse::CardKind::Cloze).all(|c| !result.states.iter().any(|(id, _)| *id == c.id)), "the never reviewed cloze cards stay new");
        let dropped = write(read(&dir.join(if modern { "m.apkg" } else { "o.apkg" })).unwrap(), &dir.join("Neu"), false, &HashSet::new(), |p| p.to_string_lossy().to_string()).unwrap();
        assert!(dropped.states.is_empty(), "starting over keeps everything new");
        let again: HashSet<String> = cards.iter().map(|c| c.id.clone()).collect();
        let second = write(read(&dir.join(if modern { "m.apkg" } else { "o.apkg" })).unwrap(), &target, true, &again, |p| p.to_string_lossy().to_string()).unwrap();
        assert_eq!((second.added, second.duplicates), (0, 4));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn deck(name: &str, notes: &[&str]) -> Package {
        let notes = notes.iter().enumerate().map(|(i, text)| ImportNote { text: text.to_string(), progress: vec![Progress { anki_id: i as i64, ..Default::default() }], tags: Vec::new() }).collect();
        Package { decks: BTreeMap::from([(vec![name.to_string()], notes)]), ..Default::default() }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("schrumbi-anki-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cards_with_the_same_question_all_arrive() {
        let dir = scratch("same-front");
        let result = write(deck("Vokabeln", &["Haus :: house", "Haus :: home", "Baum :: tree", "Baum :: tree"]), &dir, true, &HashSet::new(), |p| p.to_string_lossy().to_string()).unwrap();
        let note = std::fs::read_to_string(dir.join("Vokabeln.md")).unwrap();
        let cards = parse::cards_in(&note);
        assert_eq!(cards.len(), 3, "{note}");
        assert!(cards.iter().any(|c| c.back == "house") && cards.iter().any(|c| c.back == "home"), "{note}");
        assert!(cards.iter().any(|c| c.front == "Haus") && cards.iter().any(|c| c.front == "Haus (2)"), "only the second one gets a suffix: {note}");
        assert_eq!((result.added, result.duplicates), (3, 1));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_note_that_is_not_utf8_is_never_replaced_by_imported_cards() {
        let dir = scratch("latin1");
        let latin = b"Meine Notiz \xe4\xf6\xfc\n".to_vec();
        std::fs::write(dir.join("Bio.md"), &latin).unwrap();
        let result = write(deck("Bio", &["Was ist ATP? :: Energie"]), &dir, true, &HashSet::new(), |p| p.to_string_lossy().to_string()).unwrap();
        assert_eq!(std::fs::read(dir.join("Bio.md")).unwrap(), latin);
        assert_eq!(result.created.len(), 1);
        assert!(std::fs::read_to_string(&result.created[0]).unwrap().contains("Was ist ATP?"));
        std::fs::write(dir.join("Chemie.md"), "Eigenes\n").unwrap();
        let result = write(deck("Chemie", &["Was ist H2O? :: Wasser"]), &dir, true, &HashSet::new(), |p| p.to_string_lossy().to_string()).unwrap();
        assert_eq!(result.appended, vec![(dir.join("Chemie.md"), b"Eigenes\n".to_vec())], "cards added to a note are known, so undo can take them back");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_from_a_package_never_leave_the_import_folder() {
        let dir = scratch("names");
        let target = dir.join("Anki");
        let mut package = deck("..", &["F :: B"]);
        package.decks.insert(vec!["C:".to_string(), "..".to_string(), "CON".to_string()], vec![ImportNote { text: "G :: H".to_string(), progress: vec![Progress::default()], tags: Vec::new() }]);
        package.media = vec![("CON".to_string(), b"x".to_vec()), ("ok.png".to_string(), b"y".to_vec()), ("a?.png".to_string(), b"z".to_vec())];
        let result = write(package, &target, true, &HashSet::new(), |p| p.to_string_lossy().to_string()).unwrap();
        assert!(result.created.iter().all(|p| p.starts_with(&target)), "{:?}", result.created);
        assert_eq!((result.media, result.skipped_media), (1, 2));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn far_future_or_unreviewed_progress_is_taken_as_new() {
        let far = progress_of(2, 2, i64::MAX / 2, i64::MAX, 2500, 5, 0, "", 1_700_000_000, 1);
        assert_eq!(far.phase, Phase::New);
        let never = progress_of(2, 2, 100, 0, 2500, 0, 0, "", 1_700_000_000, 2);
        assert_eq!(never.phase, Phase::New);
        let real = progress_of(2, 2, 100, 30, 2500, 4, 0, "", 1_700_000_000, 3);
        assert_eq!((real.phase, real.due), (Phase::Review, 1_700_000_000 + 100 * 86_400));
    }

    #[test]
    fn reads_older_packages() {
        check(false);
    }

    #[test]
    fn reads_current_packages() {
        check(true);
    }

    #[test]
    fn reads_text_exports() {
        let dir = std::env::temp_dir().join(format!("schrumbi-anki-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Vokabeln.txt");
        std::fs::write(&file, "#separator:tab\n#html:true\nHaus\t<b>house</b>\n\"a\"\"b\"\tc\n").unwrap();
        let package = read(&file).unwrap();
        assert_eq!(package.cards, 2);
        assert_eq!(package.decks[&vec!["Vokabeln".to_string()]][0].text, "Haus :: **house**");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("schrumbi-anki-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.apkg");
        std::fs::write(&file, "kein zip").unwrap();
        assert!(read(&file).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
