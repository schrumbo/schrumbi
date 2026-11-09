use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, Point};
use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId, Stream, dictionary};

use super::highlights::{self, Highlight, HighlightPart, InkLine, Pin, SIDECAR_ID_BASE};
use super::i18n::tr;

const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
const DEPTH: usize = 64;
const PIN_SIZE: f32 = 20.;

type Entry = (ObjectId, Dictionary);

/// A change to the pages of a PDF. Page numbers count from 0.
#[derive(Debug, Clone, PartialEq)]
pub enum PageOp {
    Delete(Vec<usize>),
    Rotate { pages: Vec<usize>, clockwise: bool },
    /// The listed pages keep their order and land so that the first of them sits at index `to` of the result; the others follow it directly and the remaining pages keep their order around them. A `to` past the end puts them last.
    Move { pages: Vec<usize>, to: usize },
    /// Puts all pages of another PDF before index `at`; an `at` past the end appends them.
    Insert { at: usize, pdf: Vec<u8> },
}

/// Why a PDF was left as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    Protected,
    Signed,
    Broken,
    Empty,
}

impl EditError {
    /// Title and explanation for the person, in everyday words.
    pub fn describe(&self) -> (String, String) {
        match self {
            Self::Protected => (tr("Die PDF ist geschützt"), tr("Sie ist mit einem Passwort gesichert. Schrumbi ändert sie nicht, damit nichts kaputtgeht.")),
            Self::Signed => (tr("Die PDF ist unterschrieben"), tr("Jede Änderung würde die Unterschrift ungültig machen. Die Datei bleibt, wie sie ist.")),
            Self::Broken => (tr("Das hat nicht geklappt"), tr("Die PDF ließ sich nicht sicher ändern. Die Datei bleibt, wie sie ist.")),
            Self::Empty => (tr("Eine Seite bleibt"), tr("Eine PDF braucht mindestens eine Seite. Die Datei bleibt, wie sie ist.")),
        }
    }
}

fn each_ref(object: &mut Object, f: &mut dyn FnMut(&mut Object)) {
    match object {
        Object::Reference(_) => f(object),
        Object::Array(items) => items.iter_mut().for_each(|o| each_ref(o, f)),
        Object::Dictionary(d) => d.iter_mut().for_each(|(_, o)| each_ref(o, f)),
        Object::Stream(s) => s.dict.iter_mut().for_each(|(_, o)| each_ref(o, f)),
        _ => {}
    }
}

fn refs_of(object: &Object, out: &mut Vec<ObjectId>) {
    match object {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => items.iter().for_each(|o| refs_of(o, out)),
        Object::Dictionary(d) => d.iter().for_each(|(_, o)| refs_of(o, out)),
        Object::Stream(s) => s.dict.iter().for_each(|(_, o)| refs_of(o, out)),
        _ => {}
    }
}

fn dict_of(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn mentions_encryption(data: &[u8]) -> bool {
    data.windows(8).any(|w| w == b"/Encrypt")
}

fn is_signed(doc: &Document) -> bool {
    let form = doc.catalog().ok().and_then(|c| c.get(b"AcroForm").ok()).and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok());
    let flagged = form.and_then(|f| f.get(b"SigFlags").ok()).and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_i64().ok()).is_some_and(|flags| flags & 1 == 1);
    flagged
        || doc.objects.values().filter_map(dict_of).any(|d| d.get(b"FT").and_then(Object::as_name).is_ok_and(|n| n == b"Sig") && d.get(b"V").is_ok_and(|v| !v.is_null()))
}

fn reachable(doc: &Document) -> HashSet<ObjectId> {
    let mut stack = Vec::new();
    doc.trailer.iter().for_each(|(_, o)| refs_of(o, &mut stack));
    let mut keep = HashSet::new();
    while let Some(id) = stack.pop() {
        if keep.insert(id)
            && let Some(object) = doc.objects.get(&id)
        {
            refs_of(object, &mut stack);
        }
    }
    keep
}

fn lost_objects(doc: &Document) -> bool {
    reachable(doc).iter().any(|id| !doc.objects.contains_key(id) && doc.reference_table.get(id.0).is_some_and(|e| e.is_normal() || e.is_compressed()))
}

fn read(data: &[u8]) -> Result<Document, EditError> {
    let doc = match Document::load_mem_with_options(data, LoadOptions { strict: true, ..Default::default() }).or_else(|_| Document::load_mem(data)) {
        Ok(doc) => doc,
        Err(_) if mentions_encryption(data) => return Err(EditError::Protected),
        Err(_) => return Err(EditError::Broken),
    };
    if doc.is_encrypted() || doc.was_encrypted() {
        return Err(EditError::Protected);
    }
    if is_signed(&doc) {
        return Err(EditError::Signed);
    }
    if lost_objects(&doc) {
        return Err(EditError::Broken);
    }
    Ok(doc)
}

fn pages_root(doc: &Document) -> Result<ObjectId, EditError> {
    doc.catalog().ok().and_then(|c| c.get(b"Pages").ok()).and_then(|o| o.as_reference().ok()).ok_or(EditError::Broken)
}

fn walk(doc: &Document, id: ObjectId, inherited: &Dictionary, out: &mut Vec<Entry>, path: &mut HashSet<ObjectId>) -> Result<(), EditError> {
    if path.len() > DEPTH || !path.insert(id) {
        return Err(EditError::Broken);
    }
    let node = doc.get_dictionary(id).map_err(|_| EditError::Broken)?;
    let mut here = inherited.clone();
    for key in INHERITED {
        if let Ok(value) = node.get(key) {
            here.set(key.to_vec(), value.clone());
        }
    }
    match node.get(b"Kids") {
        Ok(kids) => {
            let kids = doc.dereference(kids).and_then(|(_, o)| o.as_array()).map_err(|_| EditError::Broken)?;
            for kid in kids {
                walk(doc, kid.as_reference().map_err(|_| EditError::Broken)?, &here, out, path)?;
            }
        }
        Err(_) => out.push((id, here)),
    }
    path.remove(&id);
    Ok(())
}

fn shown_leaves(doc: &Document, data: &[u8]) -> Result<Vec<Entry>, EditError> {
    let out = leaves(doc)?;
    if Pdf::new(Arc::new(data.to_vec())).map(|p| p.pages().len()).ok() != Some(out.len()) {
        return Err(EditError::Broken);
    }
    Ok(out)
}

fn leaves(doc: &Document) -> Result<Vec<Entry>, EditError> {
    let root = pages_root(doc)?;
    if !doc.get_dictionary(root).is_ok_and(|d| d.has(b"Kids")) {
        return Err(EditError::Broken);
    }
    let mut out = Vec::new();
    walk(doc, root, &Dictionary::new(), &mut out, &mut HashSet::new())?;
    Ok(out)
}

fn valid_set(pages: &[usize], count: usize) -> Result<HashSet<usize>, EditError> {
    if pages.iter().any(|&p| p >= count) {
        return Err(EditError::Broken);
    }
    Ok(pages.iter().copied().collect())
}

/// The old index of every page of the result, for moving `pages` (duplicates and indexes past `count` ignored) to `to`.
pub fn move_order(count: usize, pages: &[usize], to: usize) -> Vec<usize> {
    let mut seen = HashSet::new();
    let moving: Vec<usize> = pages.iter().copied().filter(|&p| p < count && seen.insert(p)).collect();
    let rest: Vec<usize> = (0..count).filter(|p| !seen.contains(p)).collect();
    let to = to.min(rest.len());
    rest[..to].iter().chain(&moving).chain(&rest[to..]).copied().collect()
}

fn number(doc: &Document, object: Option<&Object>) -> Option<f32> {
    doc.dereference(object?).ok()?.1.as_float().ok()
}

fn absorb(doc: &mut Document, other: &mut Document, extra: Vec<Entry>, carry: &mut Dictionary) -> Result<Vec<Entry>, EditError> {
    let top = doc.objects.keys().map(|k| k.0).max().unwrap_or(0).max(doc.max_id);
    let incoming = other.objects.keys().map(|k| k.0).max().unwrap_or(0);
    let end = top.checked_add(incoming).ok_or(EditError::Broken)?;
    let mut shift = |o: &mut Object| {
        if let Object::Reference((n, _)) = o {
            *n += top;
        }
    };
    for ((n, g), mut object) in std::mem::take(&mut other.objects) {
        each_ref(&mut object, &mut shift);
        doc.objects.insert((n + top, g), object);
    }
    doc.max_id = end;
    carry.iter_mut().for_each(|(_, o)| each_ref(o, &mut shift));
    Ok(extra
        .into_iter()
        .map(|((n, g), mut inherited)| {
            inherited.iter_mut().for_each(|(_, o)| each_ref(o, &mut shift));
            ((n + top, g), inherited)
        })
        .collect())
}

fn outline_kids(doc: &Document, parent: ObjectId) -> Vec<ObjectId> {
    let (mut out, mut seen) = (Vec::new(), HashSet::new());
    let mut next = doc.get_dictionary(parent).ok().and_then(|d| d.get(b"First").ok()).and_then(|o| o.as_reference().ok());
    while let Some(id) = next
        && seen.insert(id)
    {
        out.push(id);
        next = doc.get_dictionary(id).ok().and_then(|d| d.get(b"Next").ok()).and_then(|o| o.as_reference().ok());
    }
    out
}

fn points_at(doc: &Document, item: ObjectId, gone: &HashSet<ObjectId>) -> bool {
    let Ok(dict) = doc.get_dictionary(item) else { return false };
    let dest = dict.get(b"Dest").ok().or_else(|| dict.get(b"A").ok().and_then(|a| doc.dereference(a).ok()).and_then(|(_, a)| a.as_dict().ok()).and_then(|a| a.get(b"D").ok()));
    dest.and_then(|d| doc.dereference(d).ok()).and_then(|(_, d)| d.as_array().ok()).and_then(|a| a.first()).and_then(|p| p.as_reference().ok()).is_some_and(|p| gone.contains(&p))
}

fn outline_survivors(doc: &Document, parent: ObjectId, gone: &HashSet<ObjectId>, depth: usize, seen: &mut HashSet<ObjectId>, out: &mut Vec<ObjectId>) {
    if depth > DEPTH {
        return;
    }
    for kid in outline_kids(doc, parent) {
        if !seen.insert(kid) {
            continue;
        }
        if points_at(doc, kid, gone) {
            outline_survivors(doc, kid, gone, depth + 1, seen, out);
        } else {
            out.push(kid);
        }
    }
}

fn relink(doc: &mut Document, parent: ObjectId, gone: &HashSet<ObjectId>, depth: usize, seen: &mut HashSet<ObjectId>) -> i64 {
    let mut kids = Vec::new();
    outline_survivors(doc, parent, gone, depth, seen, &mut kids);
    let closed = doc.get_dictionary(parent).ok().and_then(|d| d.get(b"Count").ok()).and_then(|o| o.as_i64().ok()).is_some_and(|c| c < 0);
    let mut total = kids.len() as i64;
    for (i, &kid) in kids.iter().enumerate() {
        if depth < DEPTH {
            total += relink(doc, kid, gone, depth + 1, seen);
        }
        let Ok(dict) = doc.get_dictionary_mut(kid) else { continue };
        dict.set("Parent", parent);
        match i.checked_sub(1).and_then(|p| kids.get(p)) {
            Some(&prev) => dict.set("Prev", prev),
            None => {
                dict.remove(b"Prev");
            }
        }
        match kids.get(i + 1) {
            Some(&next) => dict.set("Next", next),
            None => {
                dict.remove(b"Next");
            }
        }
    }
    if let Ok(dict) = doc.get_dictionary_mut(parent) {
        match (kids.first(), kids.last()) {
            (Some(&first), Some(&last)) => {
                dict.set("First", first);
                dict.set("Last", last);
                dict.set("Count", if closed { -total } else { total });
            }
            _ => {
                dict.remove(b"First");
                dict.remove(b"Last");
                dict.remove(b"Count");
            }
        }
    }
    if closed { 0 } else { total }
}

fn tidy_outline(doc: &mut Document, gone: &HashSet<ObjectId>) {
    let root = doc.catalog().ok().and_then(|c| c.get(b"Outlines").ok()).and_then(|o| o.as_reference().ok());
    if let Some(root) = root {
        relink(doc, root, gone, 0, &mut HashSet::from([root]));
    }
}

fn resolved_dict(doc: &Document, object: Option<&Object>) -> Option<Dictionary> {
    doc.dereference(object?).ok()?.1.as_dict().ok().cloned()
}

fn resolved_list(doc: &Document, object: Option<&Object>) -> Vec<Object> {
    object.and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_array().ok()).cloned().unwrap_or_default()
}

fn catalog_entry(doc: &Document, key: &[u8]) -> Option<Dictionary> {
    resolved_dict(doc, doc.catalog().ok()?.get(key).ok())
}

fn put(doc: &mut Document, key: &[u8], value: Dictionary) {
    match doc.catalog().ok().and_then(|c| c.get(key).ok()).and_then(|o| o.as_reference().ok()) {
        Some(id) => {
            doc.objects.insert(id, Object::Dictionary(value));
        }
        None => {
            if let Ok(catalog) = doc.catalog_mut() {
                catalog.set(key.to_vec(), value);
            }
        }
    }
}

fn carried(other: &Document) -> Dictionary {
    let mut out = Dictionary::new();
    if let Some(form) = catalog_entry(other, b"AcroForm") {
        let fields = resolved_list(other, form.get(b"Fields").ok());
        if !fields.is_empty() {
            out.set("Fields", fields);
            for key in ["DR", "DA", "NeedAppearances"] {
                if let Ok(value) = form.get(key.as_bytes()) {
                    out.set(key, value.clone());
                }
            }
        }
    }
    if let Some(props) = catalog_entry(other, b"OCProperties") {
        let groups = resolved_list(other, props.get(b"OCGs").ok());
        if !groups.is_empty() {
            let config = resolved_dict(other, props.get(b"D").ok()).unwrap_or_default();
            let listed = |key: &[u8]| resolved_list(other, config.get(key).ok());
            let without = |list: &[Object]| groups.iter().filter(|g| !list.contains(g)).cloned().collect::<Vec<_>>();
            let (off, on) = if config.get(b"BaseState").and_then(Object::as_name).is_ok_and(|n| n == b"OFF") {
                let on = listed(b"ON");
                (without(&on), on)
            } else {
                let off = listed(b"OFF");
                let on = without(&off);
                (off, on)
            };
            out.set("OCGs", groups);
            out.set("Off", off);
            out.set("On", on);
        }
    }
    out
}

fn field_name(doc: &Document, field: &Object) -> Option<String> {
    let dict = doc.dereference(field).ok()?.1.as_dict().ok()?;
    lopdf::decode_text_string(dict.get(b"T").ok()?).ok()
}

fn merge_form(doc: &mut Document, carry: &Dictionary) {
    let Ok(Object::Array(fields)) = carry.get(b"Fields") else { return };
    let mut form = catalog_entry(doc, b"AcroForm").unwrap_or_default();
    let mut all = resolved_list(doc, form.get(b"Fields").ok());
    let mut taken: HashSet<String> = all.iter().filter_map(|f| field_name(doc, f)).collect();
    for field in fields {
        if let Object::Reference(id) = field
            && let Some(name) = field_name(doc, field)
            && !taken.insert(name.clone())
        {
            let fresh = (2..).map(|n| format!("{name} {n}")).find(|candidate| !taken.contains(candidate)).unwrap_or_default();
            if let Ok(dict) = doc.get_dictionary_mut(*id) {
                dict.set("T", lopdf::text_string(&fresh));
            }
            taken.insert(fresh);
        }
    }
    all.extend(fields.iter().cloned());
    form.set("Fields", all);
    if !form.has(b"DA")
        && let Ok(da) = carry.get(b"DA")
    {
        form.set("DA", da.clone());
    }
    if carry.get(b"NeedAppearances").is_ok_and(|v| matches!(v, Object::Boolean(true))) {
        form.set("NeedAppearances", true);
    }
    let mut resources = resolved_dict(doc, form.get(b"DR").ok()).unwrap_or_default();
    if let Some(extra) = resolved_dict(doc, carry.get(b"DR").ok()) {
        for (category, value) in extra.iter() {
            match (resolved_dict(doc, resources.get(category).ok()), resolved_dict(doc, Some(value))) {
                (Some(mut mine), Some(theirs)) => {
                    let fresh: Vec<(Vec<u8>, Object)> = theirs.iter().filter(|(name, _)| !mine.has(name)).map(|(name, item)| (name.clone(), item.clone())).collect();
                    fresh.into_iter().for_each(|(name, item)| mine.set(name, item));
                    resources.set(category.clone(), mine);
                }
                _ if !resources.has(category) => resources.set(category.clone(), value.clone()),
                _ => {}
            }
        }
    }
    if !resources.is_empty() {
        form.set("DR", resources);
    }
    put(doc, b"AcroForm", form);
}

fn merge_layers(doc: &mut Document, carry: &Dictionary) {
    let Ok(Object::Array(groups)) = carry.get(b"OCGs") else { return };
    let mut props = catalog_entry(doc, b"OCProperties").unwrap_or_default();
    let mut all = resolved_list(doc, props.get(b"OCGs").ok());
    all.extend(groups.iter().cloned());
    props.set("OCGs", all);
    let mut config = resolved_dict(doc, props.get(b"D").ok()).unwrap_or_default();
    let list = |key: &str| carry.get(key.as_bytes()).ok().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default();
    let (off, on) = (list("Off"), list("On"));
    if !off.is_empty() {
        let mut all = resolved_list(doc, config.get(b"OFF").ok());
        all.extend(off);
        config.set("OFF", all);
    }
    if config.get(b"BaseState").and_then(Object::as_name).is_ok_and(|n| n == b"OFF") && !on.is_empty() {
        let mut all = resolved_list(doc, config.get(b"ON").ok());
        all.extend(on);
        config.set("ON", all);
    }
    props.set("D", config);
    put(doc, b"OCProperties", props);
}

fn annots_of(doc: &Document, page: ObjectId) -> Vec<ObjectId> {
    let list = resolved_list(doc, doc.get_dictionary(page).ok().and_then(|p| p.get(b"Annots").ok()));
    list.iter().filter_map(|o| o.as_reference().ok()).collect()
}

fn live_field(doc: &mut Document, field: ObjectId, dead: &HashSet<ObjectId>, seen: &mut HashSet<ObjectId>, depth: usize) -> bool {
    if dead.contains(&field) {
        return false;
    }
    if depth > DEPTH || !seen.insert(field) {
        return true;
    }
    let Some(kids) = doc.get_dictionary(field).ok().map(|d| resolved_list(doc, d.get(b"Kids").ok())).filter(|k| !k.is_empty()) else { return true };
    let keep: Vec<Object> = kids.iter().filter(|k| k.as_reference().map_or(true, |id| live_field(doc, id, dead, seen, depth + 1))).cloned().collect();
    if keep.len() != kids.len()
        && let Ok(dict) = doc.get_dictionary_mut(field)
    {
        dict.set("Kids", keep.clone());
    }
    !keep.is_empty()
}

fn drop_fields(doc: &mut Document, gone: &HashSet<ObjectId>, kept: &HashSet<ObjectId>) {
    let staying: HashSet<ObjectId> = kept.iter().flat_map(|&p| annots_of(doc, p)).collect();
    let dead: HashSet<ObjectId> = gone.iter().flat_map(|&p| annots_of(doc, p)).filter(|id| !staying.contains(id)).collect();
    let Some(mut form) = catalog_entry(doc, b"AcroForm") else { return };
    if dead.is_empty() {
        return;
    }
    let fields = resolved_list(doc, form.get(b"Fields").ok());
    let mut seen = HashSet::new();
    let keep: Vec<Object> = fields.iter().filter(|f| f.as_reference().map_or(true, |id| live_field(doc, id, &dead, &mut seen, 0))).cloned().collect();
    if keep.len() != fields.len() {
        form.set("Fields", keep);
        put(doc, b"AcroForm", form);
    }
}

fn cut(doc: &mut Document, gone: &HashSet<ObjectId>) {
    if gone.is_empty() {
        return;
    }
    tidy_outline(doc, gone);
    for id in gone {
        doc.objects.remove(id);
    }
    let mut null = |o: &mut Object| {
        if let Object::Reference(id) = *o
            && gone.contains(&id)
        {
            *o = Object::Null;
        }
    };
    doc.objects.values_mut().for_each(|o| each_ref(o, &mut null));
    doc.trailer.iter_mut().for_each(|(_, o)| each_ref(o, &mut null));
}

fn prune(doc: &mut Document) {
    let keep = reachable(doc);
    doc.objects.retain(|id, _| keep.contains(id));
}

fn install(doc: &mut Document, before: HashSet<ObjectId>, entries: Vec<Entry>, turn: Option<&(HashSet<usize>, bool)>) -> Result<(), EditError> {
    let root = pages_root(doc)?;
    doc.max_id = doc.max_id.max(doc.objects.keys().map(|k| k.0).max().unwrap_or(0));
    let mut counts: HashMap<ObjectId, usize> = HashMap::new();
    entries.iter().for_each(|(id, _)| *counts.entry(*id).or_default() += 1);
    let originals: HashMap<ObjectId, Object> = counts.into_iter().filter(|(_, n)| *n > 1).filter_map(|(id, _)| Some((id, doc.objects.get(&id)?.clone()))).collect();
    let mut seen = HashSet::new();
    let mut kept = Vec::with_capacity(entries.len());
    for (position, (id, inherited)) in entries.into_iter().enumerate() {
        let id = if seen.insert(id) {
            id
        } else {
            let copy = originals.get(&id).cloned().ok_or(EditError::Broken)?;
            doc.try_add_object(copy).map_err(|_| EditError::Broken)?
        };
        let current = number(doc, doc.get_dictionary(id).ok().and_then(|p| p.get(b"Rotate").ok())).or_else(|| number(doc, inherited.get(b"Rotate").ok()));
        let page = doc.get_dictionary_mut(id).map_err(|_| EditError::Broken)?;
        for key in INHERITED {
            if !page.has(key)
                && let Ok(value) = inherited.get(key)
            {
                page.set(key.to_vec(), value.clone());
            }
        }
        page.set("Parent", root);
        if let Some((which, clockwise)) = turn
            && which.contains(&position)
        {
            let degrees = current.map_or(0, |c| c as i64);
            let degrees = if degrees % 90 == 0 { degrees } else { 0 };
            page.set("Rotate", (degrees + if *clockwise { 90 } else { -90 }).rem_euclid(360));
        }
        kept.push(id);
    }
    let count = kept.len() as i64;
    let tree = doc.get_dictionary_mut(root).map_err(|_| EditError::Broken)?;
    tree.set("Kids", kept.iter().map(|&id| Object::Reference(id)).collect::<Vec<_>>());
    tree.set("Count", count);
    INHERITED.iter().for_each(|key| {
        tree.remove(key);
    });
    let kept: HashSet<ObjectId> = kept.into_iter().collect();
    let gone: HashSet<ObjectId> = before.difference(&kept).copied().collect();
    drop_fields(doc, &gone, &kept);
    cut(doc, &gone);
    prune(doc);
    Ok(())
}

fn check(bytes: Vec<u8>, expected: usize) -> Result<Vec<u8>, EditError> {
    let shared = Arc::new(bytes);
    let pages = Pdf::new(shared.clone()).map(|pdf| pdf.pages().len());
    if pages.ok() != Some(expected) {
        return Err(EditError::Broken);
    }
    Arc::try_unwrap(shared).map_err(|_| EditError::Broken)
}

fn finish(mut doc: Document, expected: usize) -> Result<Vec<u8>, EditError> {
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|_| EditError::Broken)?;
    check(out, expected)
}

/// Applies a page change and returns the new file. Pages that are deleted leave the file completely. The result is only returned when it opens and has exactly the expected number of pages.
pub fn apply(data: &[u8], op: &PageOp) -> Result<Vec<u8>, EditError> {
    let mut doc = read(data)?;
    let mut entries = shown_leaves(&doc, data)?;
    let count = entries.len();
    let before: HashSet<ObjectId> = entries.iter().map(|(id, _)| *id).collect();
    let mut turn = None;
    match op {
        PageOp::Delete(pages) => {
            let gone = valid_set(pages, count)?;
            if gone.is_empty() || gone.len() == count {
                return Err(EditError::Empty);
            }
            entries = entries.into_iter().enumerate().filter(|(i, _)| !gone.contains(i)).map(|(_, e)| e).collect();
        }
        PageOp::Rotate { pages, clockwise } => {
            let which = valid_set(pages, count)?;
            if which.is_empty() {
                return Err(EditError::Empty);
            }
            turn = Some((which, *clockwise));
        }
        PageOp::Move { pages, to } => {
            if valid_set(pages, count)?.is_empty() {
                return Err(EditError::Empty);
            }
            let mut slots: Vec<Option<Entry>> = entries.into_iter().map(Some).collect();
            entries = move_order(count, pages, *to).into_iter().filter_map(|i| slots[i].take()).collect();
        }
        PageOp::Insert { at, pdf } => {
            let mut other = read(pdf)?;
            let extra = shown_leaves(&other, pdf)?;
            if extra.is_empty() {
                return Err(EditError::Empty);
            }
            let mut carry = carried(&other);
            let extra = absorb(&mut doc, &mut other, extra, &mut carry)?;
            merge_form(&mut doc, &carry);
            merge_layers(&mut doc, &carry);
            entries.splice((*at).min(count)..(*at).min(count), extra);
        }
    }
    if !matches!(op, PageOp::Rotate { .. })
        && let Ok(catalog) = doc.catalog_mut()
    {
        catalog.remove(b"PageLabels");
    }
    let expected = entries.len();
    install(&mut doc, before, entries, turn.as_ref())?;
    finish(doc, expected)
}

fn reshape(list: &[Highlight], page: &dyn Fn(usize) -> Option<usize>, point: &dyn Fn(usize, [f32; 2]) -> [f32; 2], rect: &dyn Fn(usize, [f32; 4]) -> [f32; 4]) -> Vec<Highlight> {
    let mut out: Vec<(Highlight, bool)> = list
        .iter()
        .filter_map(|h| {
            let mut out = h.clone();
            out.parts = h.parts.iter().filter_map(|p| Some(HighlightPart { page: page(p.page)?, rects: p.rects.iter().map(|r| rect(p.page, *r)).collect() })).collect();
            out.pin = h.pin.and_then(|p| {
                let [x, y, _, _] = rect(p.page, [p.x, p.y, PIN_SIZE, PIN_SIZE]);
                Some(Pin { page: page(p.page)?, x, y })
            });
            out.ink = h.ink.iter().filter_map(|l| Some(InkLine { page: page(l.page)?, width: l.width, points: l.points.iter().map(|q| point(l.page, *q)).collect() })).collect();
            let lost = out.parts.len() < h.parts.len() || (h.pin.is_some() && out.pin.is_none()) || out.ink.len() < h.ink.len();
            let empty = out.parts.is_empty() && out.pin.is_none() && out.ink.is_empty();
            let demoted = !h.parts.is_empty() && out.parts.is_empty() && h.id < SIDECAR_ID_BASE;
            (!(lost && empty)).then_some((out, demoted))
        })
        .collect();
    let mut taken: HashSet<u64> = out.iter().map(|(h, _)| h.id).collect();
    for (h, demoted) in &mut out {
        if *demoted {
            h.id = highlights::random_id(|id| taken.contains(&id));
            taken.insert(h.id);
        }
    }
    out.into_iter().map(|(h, _)| h).collect()
}

/// Carries marks over a page change. `sizes` are the display sizes of the pages before the change, `inserted` is the number of pages an insert added. A mark left with nothing after a delete is dropped.
pub fn remap(list: &[Highlight], op: &PageOp, sizes: &[(f32, f32)], inserted: usize) -> Vec<Highlight> {
    let same_point = |_: usize, p: [f32; 2]| p;
    let same_rect = |_: usize, r: [f32; 4]| r;
    match op {
        PageOp::Delete(pages) => {
            let mut gone: Vec<usize> = pages.to_vec();
            gone.sort_unstable();
            gone.dedup();
            reshape(list, &|p| gone.binary_search(&p).is_err().then(|| p - gone.partition_point(|&g| g < p)), &same_point, &same_rect)
        }
        PageOp::Move { pages, to } => {
            let mut position: Vec<usize> = (0..sizes.len()).collect();
            for (new, old) in move_order(sizes.len(), pages, *to).into_iter().enumerate() {
                position[old] = new;
            }
            reshape(list, &|p| Some(position.get(p).copied().unwrap_or(p)), &same_point, &same_rect)
        }
        PageOp::Insert { at, .. } => {
            let at = (*at).min(sizes.len());
            reshape(list, &|p| Some(if p >= at { p + inserted } else { p }), &same_point, &same_rect)
        }
        PageOp::Rotate { pages, clockwise } => {
            let turned: HashSet<usize> = pages.iter().copied().collect();
            let size = |p: usize| if turned.contains(&p) { sizes.get(p).copied() } else { None };
            let clockwise = *clockwise;
            let point = |p: usize, [x, y]: [f32; 2]| match size(p) {
                Some((_, h)) if clockwise => [h - y, x],
                Some((w, _)) => [y, w - x],
                None => [x, y],
            };
            let rect = |p: usize, [x, y, rw, rh]: [f32; 4]| match size(p) {
                Some((_, h)) if clockwise => [h - y - rh, x, rh, rw],
                Some((w, _)) => [y, w - x - rw, rh, rw],
                None => [x, y, rw, rh],
            };
            reshape(list, &Some, &point, &rect)
        }
    }
}

fn num(v: f32) -> String {
    format!("{v}")
}

fn user(inverse: &Affine, x: f32, y: f32) -> [f32; 2] {
    let p = *inverse * Point::new(f64::from(x), f64::from(y));
    [p.x as f32, p.y as f32]
}

fn bounds(points: &[[f32; 2]], pad: f32) -> [f32; 4] {
    let min = |i: usize| points.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min);
    let max = |i: usize| points.iter().map(|p| p[i]).fold(f32::NEG_INFINITY, f32::max);
    [min(0) - pad, min(1) - pad, max(0) + pad, max(1) + pad]
}

fn reals(values: &[f32]) -> Object {
    Object::Array(values.iter().map(|&v| Object::Real(v)).collect())
}

fn appearance(doc: &mut Document, rect: [f32; 4], blend: bool, content: String) -> Option<ObjectId> {
    let mut dict = dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => reals(&rect) };
    if blend {
        dict.set("Resources", dictionary! { "ExtGState" => dictionary! { "GS0" => dictionary! { "Type" => "ExtGState", "BM" => "Multiply" } } });
    }
    doc.try_add_object(Stream::new(dict, content.into_bytes())).ok()
}

struct Marker<'a> {
    color: [f32; 3],
    note: Option<Object>,
    page: ObjectId,
    inverse: &'a Affine,
}

impl Marker<'_> {
    fn annotation(&self, subtype: &str, rect: [f32; 4], flags: i64, ap: Option<ObjectId>) -> Dictionary {
        let mut dict = dictionary! { "Type" => "Annot", "Subtype" => subtype, "Rect" => reals(&rect), "F" => flags, "C" => reals(&self.color), "P" => self.page };
        if let Some(note) = &self.note {
            dict.set("Contents", note.clone());
        }
        if let Some(ap) = ap {
            dict.set("AP", dictionary! { "N" => ap });
        }
        dict
    }

    fn rgb(&self) -> String {
        self.color.iter().map(|c| num(*c)).collect::<Vec<_>>().join(" ")
    }

    fn corners(&self, x: f32, y: f32, w: f32, h: f32) -> [[f32; 2]; 4] {
        [user(self.inverse, x, y), user(self.inverse, x + w, y), user(self.inverse, x, y + h), user(self.inverse, x + w, y + h)]
    }

    fn text(&self, doc: &mut Document, part: &HighlightPart) -> Option<Dictionary> {
        let quads: Vec<[[f32; 2]; 4]> = part.rects.iter().filter(|r| r.iter().all(|v| v.is_finite())).map(|r| self.corners(r[0], r[1], r[2], r[3])).collect();
        let all: Vec<[f32; 2]> = quads.iter().flatten().copied().collect();
        if quads.is_empty() || !all.iter().flatten().all(|v| v.is_finite()) {
            return None;
        }
        let rect = bounds(&all, 0.);
        let mut content = format!("/GS0 gs {} rg\n", self.rgb());
        for q in &quads {
            content += &format!("{} {} m {} {} l {} {} l {} {} l h f\n", num(q[0][0]), num(q[0][1]), num(q[1][0]), num(q[1][1]), num(q[3][0]), num(q[3][1]), num(q[2][0]), num(q[2][1]));
        }
        let ap = appearance(doc, rect, true, content)?;
        let mut dict = self.annotation("Highlight", rect, 4, Some(ap));
        dict.set("QuadPoints", reals(&all.iter().flatten().copied().collect::<Vec<f32>>()));
        Some(dict)
    }

    fn pin(&self, pin: &Pin) -> Option<Dictionary> {
        let corners = self.corners(pin.x, pin.y, PIN_SIZE, PIN_SIZE);
        if !corners.iter().flatten().all(|v| v.is_finite()) {
            return None;
        }
        let mut dict = self.annotation("Text", bounds(&corners, 0.), 28, None);
        dict.set("Name", "Comment");
        dict.set("Open", false);
        Some(dict)
    }

    fn ink(&self, doc: &mut Document, line: &InkLine) -> Option<Dictionary> {
        let points: Vec<[f32; 2]> = line.points.iter().filter(|p| p.iter().all(|v| v.is_finite())).map(|p| user(self.inverse, p[0], p[1])).collect();
        if points.is_empty() || !line.width.is_finite() || !points.iter().flatten().all(|v| v.is_finite()) {
            return None;
        }
        let rect = bounds(&points, line.width / 2.);
        let mut content = format!("{} RG 1 J 1 j {} w\n{} {} m\n", self.rgb(), num(line.width), num(points[0][0]), num(points[0][1]));
        for p in points.iter().skip(1).chain(points.iter().take(1).filter(|_| points.len() == 1)) {
            content += &format!("{} {} l\n", num(p[0]), num(p[1]));
        }
        content += "S\n";
        let ap = appearance(doc, rect, false, content)?;
        let mut dict = self.annotation("Ink", rect, 4, Some(ap));
        dict.set("InkList", Object::Array(vec![reals(&points.iter().flatten().copied().collect::<Vec<f32>>())]));
        dict.set("BS", dictionary! { "W" => line.width, "S" => "S" });
        Some(dict)
    }
}

/// A copy of the PDF with the marks as real annotations that other programs show and keep. The original stays as it is, existing annotations stay too. `tint` turns a mark's color name into RGB.
pub fn with_annotations(data: &[u8], list: &[Highlight], tint: &dyn Fn(&str) -> [u8; 3]) -> Result<Vec<u8>, EditError> {
    let mut doc = read(data)?;
    let ids: Vec<ObjectId> = leaves(&doc)?.into_iter().map(|(id, _)| id).collect();
    let pdf = Pdf::new(Arc::new(data.to_vec())).map_err(|_| EditError::Broken)?;
    if pdf.pages().len() != ids.len() {
        return Err(EditError::Broken);
    }
    let inverses: Vec<Affine> = pdf.pages().iter().map(|p| p.initial_transform(true).to_kurbo().inverse()).collect();
    let mut added: HashMap<ObjectId, Vec<Object>> = HashMap::new();
    for h in list {
        let rgb = tint(&h.color);
        let color = [f32::from(rgb[0]) / 255., f32::from(rgb[1]) / 255., f32::from(rgb[2]) / 255.];
        let note = (!h.note.is_empty()).then(|| lopdf::text_string(&h.note));
        let marker = |page: usize| Some(Marker { color, note: note.clone(), page: *ids.get(page)?, inverse: inverses.get(page)? });
        let mut made: Vec<(ObjectId, Dictionary)> = Vec::new();
        for part in &h.parts {
            if let Some(m) = marker(part.page)
                && let Some(dict) = m.text(&mut doc, part)
            {
                made.push((m.page, dict));
            }
        }
        if let Some(pin) = &h.pin
            && let Some(m) = marker(pin.page)
            && let Some(dict) = m.pin(pin)
        {
            made.push((m.page, dict));
        }
        for line in &h.ink {
            if let Some(m) = marker(line.page)
                && let Some(dict) = m.ink(&mut doc, line)
            {
                made.push((m.page, dict));
            }
        }
        for (page, dict) in made {
            let id = doc.try_add_object(dict).map_err(|_| EditError::Broken)?;
            added.entry(page).or_default().push(Object::Reference(id));
        }
    }
    for (page, annots) in added {
        let list = doc.get_dictionary(page).ok().and_then(|p| p.get(b"Annots").ok()).cloned();
        match list {
            Some(Object::Reference(id)) => doc.get_object_mut(id).and_then(Object::as_array_mut).map_err(|_| EditError::Broken)?.extend(annots),
            Some(Object::Array(mut existing)) => {
                existing.extend(annots);
                doc.get_dictionary_mut(page).map_err(|_| EditError::Broken)?.set("Annots", existing);
            }
            _ => doc.get_dictionary_mut(page).map_err(|_| EditError::Broken)?.set("Annots", annots),
        }
    }
    finish(doc, ids.len())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::core::pdf_render::PdfDoc;
    use lopdf::content::{Content, Operation};
    use lopdf::{EncryptionState, EncryptionVersion, Permissions};

    fn content(doc: &mut Document, text: &str) -> ObjectId {
        let ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ];
        doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()))
    }

    fn leaf(doc: &mut Document, parent: ObjectId, text: &str) -> ObjectId {
        let contents = content(doc, text);
        doc.add_object(dictionary! { "Type" => "Page", "Parent" => parent, "Contents" => contents })
    }

    fn finish_doc(mut doc: Document, root: ObjectId, outline: &[(&str, ObjectId)]) -> Vec<u8> {
        let outlines = doc.new_object_id();
        let items: Vec<ObjectId> = outline.iter().map(|(title, page)| doc.add_object(dictionary! { "Title" => Object::string_literal(*title), "Parent" => outlines, "Dest" => vec![(*page).into(), "Fit".into()] })).collect();
        for pair in items.windows(2) {
            doc.get_object_mut(pair[0]).unwrap().as_dict_mut().unwrap().set("Next", pair[1]);
            doc.get_object_mut(pair[1]).unwrap().as_dict_mut().unwrap().set("Prev", pair[0]);
        }
        doc.objects.insert(outlines, Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => items[0], "Last" => *items.last().unwrap(), "Count" => items.len() as i64 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root, "Outlines" => outlines });
        doc.trailer.set("Root", catalog);
        doc.trailer.set("ID", Object::Array(vec![Object::string_literal(vec![1u8; 16]), Object::string_literal(vec![2u8; 16])]));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn font(doc: &mut Document) -> ObjectId {
        doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" })
    }

    fn media(w: i64, h: i64) -> Object {
        vec![0.into(), 0.into(), w.into(), h.into()].into()
    }

    /// Pages 0 to 2 sit under a middle node that holds Resources, MediaBox and Rotate 90; page 1 overrides Rotate. Page 3 is plain and under the root.
    fn fixture() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let mid = doc.new_object_id();
        let font = font(&mut doc);
        let one = leaf(&mut doc, mid, "Erste Seite");
        let two = leaf(&mut doc, mid, "Zweite Seite");
        let three = leaf(&mut doc, mid, "Dritte GEHEIM Seite");
        let four = leaf(&mut doc, root, "Vierte Regenbogen Seite");
        doc.get_object_mut(two).unwrap().as_dict_mut().unwrap().set("Rotate", 0);
        let list = doc.add_object(vec![Object::Dictionary(dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![0.into(), 0.into(), 50.into(), 50.into()], "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.org") } })]);
        doc.get_object_mut(two).unwrap().as_dict_mut().unwrap().set("Annots", list);
        let link = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![0.into(), 0.into(), 50.into(), 50.into()], "Dest" => vec![three.into(), "Fit".into()] };
        doc.get_object_mut(one).unwrap().as_dict_mut().unwrap().set("Annots", vec![Object::Dictionary(link)]);
        let own = dictionary! { "Font" => dictionary! { "F1" => font } };
        let four_dict = doc.get_object_mut(four).unwrap().as_dict_mut().unwrap();
        four_dict.set("MediaBox", media(612, 792));
        four_dict.set("Resources", own.clone());
        doc.objects.insert(mid, Object::Dictionary(dictionary! { "Type" => "Pages", "Parent" => root, "Kids" => vec![one.into(), two.into(), three.into()], "Count" => 3, "MediaBox" => media(612, 792), "Rotate" => 90, "Resources" => own }));
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![mid.into(), four.into()], "Count" => 4 }));
        finish_doc(doc, root, &[("Kapitel Eins", one), ("Kapitel Drei", three)])
    }

    fn appendix() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let mid = doc.new_object_id();
        let font = font(&mut doc);
        let a = leaf(&mut doc, mid, "Anhang A");
        let b = leaf(&mut doc, mid, "Anhang B");
        doc.objects.insert(mid, Object::Dictionary(dictionary! { "Type" => "Pages", "Parent" => root, "Kids" => vec![a.into(), b.into()], "Count" => 2, "MediaBox" => media(300, 400), "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } }));
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![mid.into()], "Count" => 2 }));
        finish_doc(doc, root, &[("Anhang-Kapitel", a)])
    }

    fn open(bytes: &[u8]) -> PdfDoc {
        PdfDoc::from_data(Arc::new(bytes.to_vec())).unwrap()
    }

    fn texts(bytes: &[u8]) -> Vec<String> {
        let doc = open(bytes);
        (0..doc.page_count()).map(|i| doc.page_text(i)).map(|t| t.slice(0, t.glyph_count()).replace('\n', "")).collect()
    }

    fn sizes(bytes: &[u8]) -> Vec<(f32, f32)> {
        let doc = open(bytes);
        (0..doc.page_count()).map(|i| doc.page_size(i).unwrap()).collect()
    }

    fn page_dicts(bytes: &[u8]) -> Vec<Dictionary> {
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter().map(|id| doc.get_dictionary(id).unwrap().clone()).collect()
    }

    fn rotations(bytes: &[u8]) -> Vec<Option<i64>> {
        page_dicts(bytes).iter().map(|d| d.get(b"Rotate").ok().and_then(|o| o.as_i64().ok())).collect()
    }

    fn contains(bytes: &[u8], needle: &[u8]) -> bool {
        bytes.windows(needle.len()).any(|w| w == needle)
    }

    fn encrypted(bytes: &[u8], user: &str) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        let state = EncryptionState::try_from(EncryptionVersion::V2 { document: &doc, owner_password: "owner", user_password: user, key_length: 128, permissions: Permissions::all() }).unwrap();
        doc.encrypt(&state).unwrap();
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn with_catalog(bytes: &[u8], key: &str, value: Object) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        doc.catalog_mut().unwrap().set(key, value);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn with_field(bytes: &[u8], signed: bool) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        let mut field = dictionary! { "FT" => "Sig", "T" => Object::string_literal("Unterschrift") };
        if signed {
            field.set("V", dictionary! { "Type" => "Sig", "Filter" => "Adobe.PPKLite" });
        }
        let id = doc.add_object(field);
        doc.catalog_mut().unwrap().set("AcroForm", dictionary! { "Fields" => vec![id.into()] });
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn mark(id: u64, page: usize) -> Highlight {
        Highlight { id, color: "yellow".into(), text: "Wort".into(), parts: vec![HighlightPart { page, rects: vec![[10., 20., 30., 5.]] }], ..Default::default() }
    }

    fn near(a: &[f32], b: &[f32]) {
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01), "{a:?} vs {b:?}");
    }

    #[test]
    fn the_fixture_reads_as_designed() {
        let bytes = fixture();
        assert_eq!(texts(&bytes), ["Erste Seite", "Zweite Seite", "Dritte GEHEIM Seite", "Vierte Regenbogen Seite"]);
        assert_eq!(sizes(&bytes), [(792., 612.), (612., 792.), (792., 612.), (612., 792.)]);
    }

    #[test]
    fn deleting_removes_the_pages_and_their_content_from_the_file() {
        let out = apply(&fixture(), &PageOp::Delete(vec![2])).unwrap();
        assert_eq!(texts(&out), ["Erste Seite", "Zweite Seite", "Vierte Regenbogen Seite"]);
        assert_eq!(sizes(&out), [(792., 612.), (612., 792.), (612., 792.)]);
        assert!(!contains(&out, b"GEHEIM"), "a deleted page must not stay in the file");
        assert!(!contains(&out, b"Dritte"));
        let meta = open(&out).meta(&out);
        assert!(meta.outline.iter().any(|e| e.title == "Kapitel Eins" && e.page == 0), "{:?}", meta.outline);
    }

    #[test]
    fn deleting_the_first_page_keeps_the_inherited_attributes_of_the_others() {
        let out = apply(&fixture(), &PageOp::Delete(vec![0, 0])).unwrap();
        assert_eq!(texts(&out), ["Zweite Seite", "Dritte GEHEIM Seite", "Vierte Regenbogen Seite"]);
        assert_eq!(sizes(&out), [(612., 792.), (792., 612.), (612., 792.)]);
        assert_eq!(open(&out).meta(&out).outline.iter().map(|e| (e.title.as_str(), e.page)).filter(|(t, _)| *t == "Kapitel Drei").collect::<Vec<_>>(), [("Kapitel Drei", 1)]);
    }

    #[test]
    fn deleting_never_empties_the_file_and_refuses_bad_pages() {
        assert_eq!(apply(&fixture(), &PageOp::Delete(vec![0, 1, 2, 3])), Err(EditError::Empty));
        assert_eq!(apply(&fixture(), &PageOp::Delete(vec![])), Err(EditError::Empty));
        assert_eq!(apply(&fixture(), &PageOp::Delete(vec![4])), Err(EditError::Broken));
        assert_eq!(apply(&fixture(), &PageOp::Rotate { pages: vec![], clockwise: true }), Err(EditError::Empty));
        assert_eq!(apply(&fixture(), &PageOp::Move { pages: vec![], to: 0 }), Err(EditError::Empty));
        assert_eq!(apply(&fixture(), &PageOp::Move { pages: vec![9], to: 0 }), Err(EditError::Broken));
    }

    #[test]
    fn rotating_writes_the_effective_rotation_onto_the_page() {
        let out = apply(&fixture(), &PageOp::Rotate { pages: vec![0, 1, 3, 3], clockwise: true }).unwrap();
        assert_eq!(rotations(&out), [Some(180), Some(90), Some(90), Some(90)]);
        assert_eq!(sizes(&out), [(612., 792.), (792., 612.), (792., 612.), (792., 612.)]);
        assert_eq!(texts(&out), texts(&fixture()));
        let back = apply(&fixture(), &PageOp::Rotate { pages: vec![0, 1], clockwise: false }).unwrap();
        assert_eq!(rotations(&back), [Some(0), Some(270), Some(90), None]);
    }

    #[test]
    fn rotating_four_times_returns_to_the_start() {
        let mut bytes = fixture();
        for _ in 0..4 {
            bytes = apply(&bytes, &PageOp::Rotate { pages: vec![1], clockwise: true }).unwrap();
        }
        assert_eq!(rotations(&bytes)[1], Some(0));
        assert_eq!(sizes(&bytes), sizes(&fixture()));
    }

    #[test]
    fn moving_places_the_first_moved_page_at_the_target_index() {
        assert_eq!(move_order(5, &[1, 2], 3), [0, 3, 4, 1, 2]);
        assert_eq!(move_order(5, &[4], 0), [4, 0, 1, 2, 3]);
        assert_eq!(move_order(5, &[0], 99), [1, 2, 3, 4, 0]);
        assert_eq!(move_order(5, &[3, 1], 1), [0, 3, 1, 2, 4]);
        assert_eq!(move_order(5, &[2, 2, 9], 0), [2, 0, 1, 3, 4]);
        assert_eq!(move_order(3, &[2, 0, 1], 1), [2, 0, 1]);
        let out = apply(&fixture(), &PageOp::Move { pages: vec![2, 0], to: 1 }).unwrap();
        assert_eq!(texts(&out), ["Zweite Seite", "Dritte GEHEIM Seite", "Erste Seite", "Vierte Regenbogen Seite"]);
        assert_eq!(sizes(&out), [(612., 792.), (792., 612.), (792., 612.), (612., 792.)]);
        let meta = open(&out).meta(&out);
        assert!(meta.outline.iter().any(|e| e.title == "Kapitel Drei" && e.page == 1) && meta.outline.iter().any(|e| e.title == "Kapitel Eins" && e.page == 2), "{:?}", meta.outline);
    }

    #[test]
    fn inserting_brings_the_pages_of_another_file_with_their_own_attributes() {
        let out = apply(&fixture(), &PageOp::Insert { at: 1, pdf: appendix() }).unwrap();
        assert_eq!(texts(&out), ["Erste Seite", "Anhang A", "Anhang B", "Zweite Seite", "Dritte GEHEIM Seite", "Vierte Regenbogen Seite"]);
        assert_eq!(sizes(&out), [(792., 612.), (300., 400.), (300., 400.), (612., 792.), (792., 612.), (612., 792.)]);
        assert!(!contains(&out, b"Anhang-Kapitel"), "the outline of the inserted file is left out");
        let meta = open(&out).meta(&out);
        assert!(meta.outline.iter().any(|e| e.title == "Kapitel Drei" && e.page == 4), "{:?}", meta.outline);
        let end = apply(&fixture(), &PageOp::Insert { at: 99, pdf: appendix() }).unwrap();
        assert_eq!(texts(&end)[4..], ["Anhang A", "Anhang B"]);
        let start = apply(&fixture(), &PageOp::Insert { at: 0, pdf: appendix() }).unwrap();
        assert_eq!(texts(&start)[..3], ["Anhang A", "Anhang B", "Erste Seite"]);
    }

    #[test]
    fn a_page_listed_twice_is_refused_because_the_viewer_shows_it_once() {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let font = font(&mut doc);
        let a = leaf(&mut doc, root, "Eins");
        let b = leaf(&mut doc, root, "Zwei");
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![a.into(), b.into(), a.into()], "Count" => 3, "MediaBox" => media(612, 792), "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } }));
        let bytes = finish_doc(doc, root, &[("A", a)]);
        assert_eq!(PdfDoc::from_data(Arc::new(bytes.clone())).map(|d| d.page_count()), Some(2));
        assert_eq!(apply(&bytes, &PageOp::Delete(vec![1])), Err(EditError::Broken));
        assert_eq!(apply(&bytes, &PageOp::Rotate { pages: vec![0], clockwise: true }), Err(EditError::Broken));
        let host = fixture();
        assert_eq!(apply(&host, &PageOp::Insert { at: 1, pdf: bytes.clone() }), Err(EditError::Broken));
    }

    #[test]
    fn a_page_written_inline_in_the_page_list_is_refused() {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let font = font(&mut doc);
        let a = leaf(&mut doc, root, "Eins");
        let inline = Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => root });
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![a.into(), inline], "Count" => 2, "MediaBox" => media(612, 792), "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } }));
        let bytes = finish_doc(doc, root, &[("A", a)]);
        assert_eq!(apply(&bytes, &PageOp::Delete(vec![0])), Err(EditError::Broken));
    }

    fn nested(levels: usize, cyclic: bool) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let font = font(&mut doc);
        let ids: Vec<ObjectId> = (0..levels).map(|_| doc.new_object_id()).collect();
        let page = leaf(&mut doc, *ids.last().unwrap_or(&root), "Tief");
        let own = dictionary! { "Font" => dictionary! { "F1" => font } };
        for (i, id) in ids.iter().enumerate() {
            let kid: Object = match ids.get(i + 1) {
                Some(next) => (*next).into(),
                None if cyclic => root.into(),
                None => page.into(),
            };
            doc.objects.insert(*id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![kid], "Count" => 1 }));
        }
        let top: Object = ids.first().map_or(page.into(), |id| (*id).into());
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![top], "Count" => 1, "MediaBox" => media(612, 792), "Resources" => own }));
        finish_doc(doc, root, &[("Tief", page)])
    }

    #[test]
    fn page_trees_that_are_absurdly_deep_or_loop_are_refused() {
        let ok = apply(&nested(60, false), &PageOp::Rotate { pages: vec![0], clockwise: true }).unwrap();
        assert_eq!(texts(&ok), ["Tief"]);
        assert_eq!(apply(&nested(63, false), &PageOp::Rotate { pages: vec![0], clockwise: true }).map(|b| texts(&b)), Ok(vec!["Tief".to_string()]));
        assert_eq!(apply(&nested(64, false), &PageOp::Rotate { pages: vec![0], clockwise: true }), Err(EditError::Broken));
        assert_eq!(apply(&nested(3, true), &PageOp::Rotate { pages: vec![0], clockwise: true }), Err(EditError::Broken));
    }

    #[test]
    fn marks_beyond_the_known_pages_stay_where_they_are() {
        let list = vec![mark(1, 7)];
        assert_eq!(remap(&list, &PageOp::Move { pages: vec![1], to: 0 }, &[(612., 792.); 3], 0), list);
        assert_eq!(remap(&list, &PageOp::Delete(vec![1]), &[(612., 792.); 3], 0)[0].parts[0].page, 6);
    }

    #[test]
    fn a_deleted_page_takes_its_outline_entry_along_and_the_entries_below_move_up() {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let font = font(&mut doc);
        let (one, two, three) = (leaf(&mut doc, root, "Eins"), leaf(&mut doc, root, "Zwei"), leaf(&mut doc, root, "Drei"));
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![one.into(), two.into(), three.into()], "Count" => 3, "MediaBox" => media(612, 792), "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } }));
        let outlines = doc.new_object_id();
        let (a, a1, a2, b) = (doc.new_object_id(), doc.new_object_id(), doc.new_object_id(), doc.new_object_id());
        let dest = |page: ObjectId| -> Object { vec![page.into(), "Fit".into()].into() };
        doc.objects.insert(a, Object::Dictionary(dictionary! { "Title" => Object::string_literal("A"), "Parent" => outlines, "Dest" => dest(three), "Next" => b, "First" => a1, "Last" => a2, "Count" => 2 }));
        doc.objects.insert(a1, Object::Dictionary(dictionary! { "Title" => Object::string_literal("A1"), "Parent" => a, "Dest" => dest(one), "Next" => a2 }));
        doc.objects.insert(a2, Object::Dictionary(dictionary! { "Title" => Object::string_literal("A2"), "Parent" => a, "Dest" => dest(three), "Prev" => a1 }));
        doc.objects.insert(b, Object::Dictionary(dictionary! { "Title" => Object::string_literal("B"), "Parent" => outlines, "Dest" => dest(two), "Prev" => a }));
        doc.objects.insert(outlines, Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => a, "Last" => b, "Count" => 4 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root, "Outlines" => outlines });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let entries = |bytes: &[u8]| open(bytes).meta(bytes).outline.iter().map(|e| (e.title.clone(), e.level, e.page)).collect::<Vec<_>>();
        assert_eq!(entries(&bytes), [("A".into(), 1, 2), ("A1".into(), 2, 0), ("A2".into(), 2, 2), ("B".into(), 1, 1)]);
        let out = apply(&bytes, &PageOp::Delete(vec![2])).unwrap();
        assert_eq!(entries(&out), [("A1".into(), 1, 0), ("B".into(), 1, 1)]);
        let out = apply(&bytes, &PageOp::Delete(vec![0, 1])).unwrap();
        assert_eq!(entries(&out), [("A".into(), 1, 0), ("A2".into(), 2, 0)]);
        let out = apply(&bytes, &PageOp::Delete(vec![0, 2])).unwrap();
        assert_eq!(entries(&out), [("B".into(), 1, 0)]);
        let doc = Document::load_mem(&out).unwrap();
        let outline = doc.catalog().unwrap().get(b"Outlines").unwrap().as_reference().unwrap();
        let root = doc.get_dictionary(outline).unwrap();
        assert_eq!(root.get(b"Count").unwrap().as_i64().unwrap(), 1);
        assert_eq!(root.get(b"First").unwrap(), root.get(b"Last").unwrap());
    }

    #[test]
    fn protected_and_signed_files_are_refused_and_so_are_inserted_ones() {
        for user in ["geheim", ""] {
            let locked = encrypted(&fixture(), user);
            assert_eq!(apply(&locked, &PageOp::Delete(vec![0])), Err(EditError::Protected), "user password {user:?}");
            assert_eq!(apply(&fixture(), &PageOp::Insert { at: 0, pdf: locked.clone() }), Err(EditError::Protected));
            assert_eq!(with_annotations(&locked, &[], &|_| [0, 0, 0]), Err(EditError::Protected));
        }
        let flagged = with_catalog(&fixture(), "AcroForm", dictionary! { "SigFlags" => 3, "Fields" => Vec::<Object>::new() }.into());
        assert_eq!(apply(&flagged, &PageOp::Delete(vec![0])), Err(EditError::Signed));
        assert_eq!(apply(&fixture(), &PageOp::Insert { at: 0, pdf: flagged }), Err(EditError::Signed));
        let signed = with_field(&fixture(), true);
        assert_eq!(apply(&signed, &PageOp::Rotate { pages: vec![0], clockwise: true }), Err(EditError::Signed));
        assert_eq!(with_annotations(&signed, &[], &|_| [0, 0, 0]), Err(EditError::Signed));
        let blank = with_field(&fixture(), false);
        assert!(apply(&blank, &PageOp::Rotate { pages: vec![0], clockwise: true }).is_ok(), "an empty signature field is no signature");
        let unflagged = with_catalog(&fixture(), "AcroForm", dictionary! { "SigFlags" => 2, "Fields" => Vec::<Object>::new() }.into());
        assert!(apply(&unflagged, &PageOp::Delete(vec![0])).is_ok());
    }

    #[test]
    fn broken_input_and_wrong_page_counts_never_come_back_as_bytes() {
        assert_eq!(apply(b"kein pdf", &PageOp::Delete(vec![0])), Err(EditError::Broken));
        assert_eq!(apply(&fixture(), &PageOp::Insert { at: 0, pdf: b"kein pdf".to_vec() }), Err(EditError::Broken));
        assert_eq!(with_annotations(b"kein pdf", &[], &|_| [0, 0, 0]), Err(EditError::Broken));
        assert_eq!(check(fixture(), 9), Err(EditError::Broken));
        assert_eq!(check(b"kein pdf".to_vec(), 1), Err(EditError::Broken));
        assert_eq!(check(fixture(), 4), Ok(fixture()));
    }

    #[test]
    fn every_refusal_explains_itself_in_words() {
        for e in [EditError::Protected, EditError::Signed, EditError::Broken, EditError::Empty] {
            let (title, body) = e.describe();
            assert!(!title.is_empty() && body.ends_with('.'));
        }
        assert_ne!(EditError::Protected.describe(), EditError::Signed.describe());
    }

    fn rich() -> Vec<Highlight> {
        vec![
            mark(1, 0),
            Highlight { id: 2, parts: vec![HighlightPart { page: 1, rects: vec![[1., 2., 3., 4.]] }, HighlightPart { page: 2, rects: vec![[5., 6., 7., 8.]] }], ..Default::default() },
            Highlight { id: 3, pin: Some(Pin { page: 2, x: 30., y: 40. }), ..Default::default() },
            Highlight { id: 4, ink: vec![InkLine { page: 1, width: 2., points: vec![[1., 1.], [9., 9.]] }], ..Default::default() },
            Highlight { id: 5, note: "bleibt".into(), parts: vec![HighlightPart { page: 1, rects: vec![[0., 0., 1., 1.]] }], ..Default::default() },
            Highlight { id: 6, ..Default::default() },
        ]
    }

    #[test]
    fn deleting_drops_what_is_on_the_page_and_shifts_the_rest() {
        let out = remap(&rich(), &PageOp::Delete(vec![1]), &[(612., 792.); 4], 0);
        assert_eq!(out.iter().map(|h| h.id).collect::<Vec<_>>(), [1, 2, 3, 6]);
        assert_eq!(out[0].parts[0].page, 0);
        assert_eq!(out[1].parts.len(), 1);
        assert_eq!(out[1].parts[0], HighlightPart { page: 1, rects: vec![[5., 6., 7., 8.]] });
        assert_eq!(out[2].pin, Some(Pin { page: 1, x: 30., y: 40. }));
        let out = remap(&rich(), &PageOp::Delete(vec![0, 2]), &[(612., 792.); 4], 0);
        assert_eq!(out.iter().map(|h| h.id).collect::<Vec<_>>(), [2, 4, 5, 6]);
        assert_eq!(out[0].parts.iter().map(|p| p.page).collect::<Vec<_>>(), [0]);
        assert_eq!(out[1].ink[0].page, 0);
    }

    #[test]
    fn a_mark_left_without_text_gets_an_id_that_the_first_release_never_hands_out() {
        use crate::core::highlights::SIDECAR_ID_BASE;
        let both = Highlight { id: 2, pin: Some(Pin { page: 2, x: 30., y: 40. }), parts: vec![HighlightPart { page: 1, rects: vec![[1., 2., 3., 4.]] }], ..Default::default() };
        let list = vec![mark(1, 0), both];
        let out = remap(&list, &PageOp::Delete(vec![1]), &[(612., 792.); 4], 0);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, 1);
        assert!(out[1].parts.is_empty() && out[1].pin.is_some());
        assert!(out[1].id >= SIDECAR_ID_BASE, "{}", out[1].id);
        let kept = remap(&list, &PageOp::Delete(vec![3]), &[(612., 792.); 4], 0);
        assert_eq!(kept, list, "a mark that still has its text keeps its id");
    }

    #[test]
    fn moving_and_inserting_shift_page_numbers() {
        let sizes = [(612., 792.); 4];
        let moved = remap(&rich(), &PageOp::Move { pages: vec![2, 0], to: 1 }, &sizes, 0);
        assert_eq!(moved[0].parts[0].page, 2);
        assert_eq!(moved[1].parts.iter().map(|p| p.page).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(moved[2].pin.unwrap().page, 1);
        assert_eq!(moved[3].ink[0].page, 0);
        let inserted = remap(&rich(), &PageOp::Insert { at: 1, pdf: Vec::new() }, &sizes, 2);
        assert_eq!(inserted[0].parts[0].page, 0);
        assert_eq!(inserted[1].parts.iter().map(|p| p.page).collect::<Vec<_>>(), [3, 4]);
        assert_eq!(inserted[2].pin.unwrap().page, 4);
        let appended = remap(&rich(), &PageOp::Insert { at: 99, pdf: Vec::new() }, &sizes, 2);
        assert_eq!(appended[1].parts[0].page, 1);
    }

    #[test]
    fn rotating_turns_points_and_rectangles_with_the_page_size() {
        let list = vec![Highlight { id: 1, parts: vec![HighlightPart { page: 0, rects: vec![[100., 200., 30., 10.]] }], pin: Some(Pin { page: 0, x: 100., y: 200. }), ink: vec![InkLine { page: 0, width: 2., points: vec![[100., 200.]] }], ..Default::default() }, mark(2, 1)];
        let cw = remap(&list, &PageOp::Rotate { pages: vec![0], clockwise: true }, &[(612., 792.), (612., 792.)], 0);
        near(&cw[0].parts[0].rects[0], &[792. - 200. - 10., 100., 10., 30.]);
        assert_eq!((cw[0].pin.unwrap().x, cw[0].pin.unwrap().y), (572., 100.));
        assert_eq!(cw[0].ink[0].points[0], [592., 100.]);
        assert_eq!(cw[1], list[1], "other pages stay");
        let ccw = remap(&list, &PageOp::Rotate { pages: vec![0], clockwise: false }, &[(612., 792.), (612., 792.)], 0);
        near(&ccw[0].parts[0].rects[0], &[200., 612. - 100. - 30., 10., 30.]);
        assert_eq!((ccw[0].pin.unwrap().x, ccw[0].pin.unwrap().y), (200., 492.));
        assert_eq!(ccw[0].ink[0].points[0], [200., 512.]);
        let unknown = remap(&list, &PageOp::Rotate { pages: vec![0], clockwise: true }, &[], 0);
        assert_eq!(unknown, list, "without a size nothing can be turned");
    }

    #[test]
    fn four_turns_are_the_identity() {
        let list = vec![Highlight { id: 1, parts: vec![HighlightPart { page: 0, rects: vec![[100., 200., 30., 10.]] }], pin: Some(Pin { page: 0, x: 11., y: 22. }), ink: vec![InkLine { page: 0, width: 2., points: vec![[5., 6.], [70., 80.]] }], ..Default::default() }];
        for clockwise in [true, false] {
            let (mut now, mut size) = (list.clone(), (612., 792.));
            for _ in 0..4 {
                now = remap(&now, &PageOp::Rotate { pages: vec![0], clockwise }, &[size], 0);
                size = (size.1, size.0);
            }
            near(&now[0].parts[0].rects[0], &list[0].parts[0].rects[0]);
            near(&[now[0].pin.unwrap().x, now[0].pin.unwrap().y], &[11., 22.]);
            near(&now[0].ink[0].points.iter().flatten().copied().collect::<Vec<_>>(), &[5., 6., 70., 80.]);
        }
        let (once, size) = (remap(&list, &PageOp::Rotate { pages: vec![0], clockwise: true }, &[(612., 792.)], 0), (792., 612.));
        let back = remap(&once, &PageOp::Rotate { pages: vec![0], clockwise: false }, &[size], 0);
        near(&back[0].parts[0].rects[0], &list[0].parts[0].rects[0]);
    }

    #[test]
    fn a_marked_word_is_still_marked_after_the_page_turns() {
        let before = fixture();
        let text = open(&before).page_text(3);
        let (first, last) = text.find("regenbogen")[0];
        let rects: Vec<[f32; 4]> = text.rects(first, last).iter().map(|r| [r.x, r.y, r.w, r.h]).collect();
        assert_eq!(rects.len(), 1);
        let list = vec![Highlight { id: 1, parts: vec![HighlightPart { page: 3, rects }], ..Default::default() }];
        for clockwise in [true, false] {
            let op = PageOp::Rotate { pages: vec![3], clockwise };
            let out = apply(&before, &op).unwrap();
            let [x, y, w, h] = remap(&list, &op, &sizes(&before), 0)[0].parts[0].rects[0];
            assert!(h > w, "a word that lay flat now stands: {w} x {h}");
            let turned = open(&out).page_text(3);
            let origins: Vec<(f32, f32)> = (first..last).map(|i| turned.rects(i, i + 1)[0]).map(|r| (r.x, r.y)).collect();
            assert!(origins.iter().all(|&(gx, gy)| gx >= x - 0.5 && gx <= x + w + 0.5 && gy >= y - 0.5 && gy <= y + h + 0.5), "{origins:?} outside {x} {y} {w} {h}");
            let (low, high) = origins.iter().fold((f32::MAX, f32::MIN), |(l, hi), o| (l.min(o.1), hi.max(o.1)));
            assert!(high - low > h * 0.6, "the glyphs fill the rectangle: {low}..{high} in {y}..{}", y + h);
        }
    }

    fn crop_rotated() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let font = font(&mut doc);
        let page = leaf(&mut doc, root, "Seite mit Beschnitt");
        let dict = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
        dict.set("MediaBox", media(612, 792));
        dict.set("CropBox", vec![50.into(), 40.into(), 562.into(), 752.into()]);
        dict.set("Rotate", 90);
        dict.set("Resources", dictionary! { "Font" => dictionary! { "F1" => font } });
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
        finish_doc(doc, root, &[("Eins", page)])
    }

    fn tint(name: &str) -> [u8; 3] {
        match name {
            "yellow" => [255, 255, 0],
            _ => [0, 0, 255],
        }
    }

    fn annots(bytes: &[u8], page: usize) -> Vec<Dictionary> {
        let doc = Document::load_mem(bytes).unwrap();
        let id = doc.page_iter().nth(page).unwrap();
        doc.get_page_annotations(id).unwrap().into_iter().cloned().collect()
    }

    fn appearance_of(bytes: &[u8], annot: &Dictionary) -> Stream {
        let doc = Document::load_mem(bytes).unwrap();
        let (_, object) = doc.dereference(annot.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap()).unwrap();
        object.as_stream().unwrap().clone()
    }

    fn floats(object: &Object) -> Vec<f32> {
        object.as_array().unwrap().iter().map(|o| o.as_float().unwrap()).collect()
    }

    fn forward(bytes: &[u8]) -> Affine {
        Pdf::new(Arc::new(bytes.to_vec())).unwrap().pages()[0].initial_transform(true).to_kurbo()
    }

    fn display(map: &Affine, p: &[f32]) -> [f32; 2] {
        let q = *map * Point::new(f64::from(p[0]), f64::from(p[1]));
        [q.x as f32, q.y as f32]
    }

    #[test]
    fn exported_marks_land_where_they_were_drawn_on_a_rotated_cropped_page() {
        let bytes = crop_rotated();
        assert_eq!(sizes(&bytes), [(712., 512.)]);
        let list = vec![
            Highlight { id: 1, color: "yellow".into(), note: "Größe prüfen".into(), parts: vec![HighlightPart { page: 0, rects: vec![[100., 50., 80., 12.], [100., 70., 40., 12.]] }], ..Default::default() },
            Highlight { id: 2, color: "blue".into(), note: "Frage".into(), pin: Some(Pin { page: 0, x: 100., y: 200. }), ..Default::default() },
            Highlight { id: 3, color: "blue".into(), ink: vec![InkLine { page: 0, width: 3., points: vec![[10., 10.], [60., 30.], [90., 10.]] }], ..Default::default() },
        ];
        let out = with_annotations(&bytes, &list, &tint).unwrap();
        assert_eq!(sizes(&out), sizes(&bytes));
        let found = annots(&out, 0);
        let subtypes: Vec<&[u8]> = found.iter().map(|a| a.get(b"Subtype").unwrap().as_name().unwrap()).collect();
        assert_eq!(subtypes, [&b"Highlight"[..], b"Text", b"Ink"]);
        let map = forward(&bytes);

        let quad = &found[0];
        let points = floats(quad.get(b"QuadPoints").unwrap());
        assert_eq!(points.len(), 16);
        let expected = [[100., 50.], [180., 50.], [100., 62.], [180., 62.], [100., 70.], [140., 70.], [100., 82.], [140., 82.]];
        for (i, want) in expected.iter().enumerate() {
            near(&display(&map, &points[i * 2..i * 2 + 2]), want);
        }
        near(&points[..2], &[50. + 50., 40. + 100.]);
        let rect = floats(quad.get(b"Rect").unwrap());
        near(&rect, &[50. + 50., 40. + 100., 50. + 82., 40. + 180.]);
        near(&floats(quad.get(b"C").unwrap()), &[1., 1., 0.]);
        assert_eq!(lopdf::decode_text_string(quad.get(b"Contents").unwrap()).unwrap(), "Größe prüfen");
        assert_eq!(quad.get(b"F").unwrap().as_i64().unwrap(), 4);
        let ap = appearance_of(&out, quad);
        let text = String::from_utf8(ap.content.clone()).unwrap();
        assert!(text.starts_with("/GS0 gs 1 1 0 rg") && text.matches(" f\n").count() == 2, "{text}");
        assert!(contains(&out, b"Multiply"));
        near(&floats(ap.dict.get(b"BBox").unwrap()), &rect);

        let pin = &found[1];
        assert_eq!(pin.get(b"Subtype").unwrap().as_name().unwrap(), b"Text");
        let rect = floats(pin.get(b"Rect").unwrap());
        near(&rect, &[50. + 200., 40. + 100., 50. + 220., 40. + 120.]);
        let a = display(&map, &rect[..2]);
        let b = display(&map, &rect[2..]);
        near(&[a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])], &[100., 200., 120., 220.]);
        assert_eq!(lopdf::decode_text_string(pin.get(b"Contents").unwrap()).unwrap(), "Frage");
        near(&floats(pin.get(b"C").unwrap()), &[0., 0., 1.]);
        assert!(!pin.has(b"AP"));

        let ink = &found[2];
        assert_eq!(ink.get(b"Subtype").unwrap().as_name().unwrap(), b"Ink");
        let lists = ink.get(b"InkList").unwrap().as_array().unwrap();
        assert_eq!(lists.len(), 1);
        let flat = floats(&lists[0]);
        for (i, want) in [[10., 10.], [60., 30.], [90., 10.]].iter().enumerate() {
            near(&display(&map, &flat[i * 2..i * 2 + 2]), want);
        }
        assert_eq!(ink.get(b"BS").unwrap().as_dict().unwrap().get(b"W").unwrap().as_float().unwrap(), 3.);
        assert!(!ink.has(b"Contents"));
        let ap = appearance_of(&out, ink);
        let text = String::from_utf8(ap.content.clone()).unwrap();
        assert!(text.contains("0 0 1 RG 1 J 1 j 3 w") && text.matches(" l\n").count() == 2 && text.ends_with("S\n"), "{text}");
    }

    #[test]
    fn exporting_keeps_existing_annotations_and_skips_what_cannot_be_placed() {
        let bytes = fixture();
        let list = vec![
            mark(1, 0),
            mark(2, 1),
            mark(3, 99),
            Highlight { id: 4, parts: vec![HighlightPart { page: 3, rects: vec![[f32::NAN, 1., 1., 1.], [1., 1., 2., 2.]] }], ..Default::default() },
            Highlight { id: 5, ink: vec![InkLine { page: 3, width: 2., points: vec![[f32::INFINITY, 1.]] }, InkLine { page: 3, width: 2., points: vec![[4., 4.]] }], ..Default::default() },
            Highlight { id: 6, pin: Some(Pin { page: 3, x: f32::NAN, y: 1. }), ..Default::default() },
            Highlight { id: 7, ..Default::default() },
        ];
        let out = with_annotations(&bytes, &list, &tint).unwrap();
        assert_eq!(texts(&out), texts(&bytes));
        let first = annots(&out, 0);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].get(b"Subtype").unwrap().as_name().unwrap(), b"Link");
        assert_eq!(first[1].get(b"Subtype").unwrap().as_name().unwrap(), b"Highlight");
        let second = annots(&out, 1);
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].get(b"Subtype").unwrap().as_name().unwrap(), b"Link");
        assert_eq!(second[1].get(b"Subtype").unwrap().as_name().unwrap(), b"Highlight");
        assert!(!second[1].has(b"Contents"));
        let last = annots(&out, 3);
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].get(b"Subtype").unwrap().as_name().unwrap(), b"Highlight");
        assert_eq!(last[1].get(b"Subtype").unwrap().as_name().unwrap(), b"Ink");
        assert!(annots(&out, 2).is_empty());
        let ap = appearance_of(&out, &last[1]);
        assert_eq!(String::from_utf8(ap.content).unwrap().matches(" l\n").count(), 1, "a single point is drawn as a dot");
        let plain = with_annotations(&bytes, &[], &tint).unwrap();
        assert_eq!(texts(&plain), texts(&bytes));
        assert_eq!(annots(&plain, 0).len(), 1);
    }

    fn wrong_offset(bytes: &[u8], id: usize) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        let at = out.windows(6).rposition(|w| w == b"\nxref\n").unwrap() + 1;
        let line = |from: usize| from + out[from..].iter().position(|&b| b == b'\n').unwrap() + 1;
        let mut slots = HashMap::new();
        let mut at = line(at);
        while let Some([start, count]) = std::str::from_utf8(&out[at..line(at) - 1]).ok().map(|h| h.split(' ').filter_map(|n| n.parse::<usize>().ok()).collect::<Vec<_>>()).and_then(|n| <[usize; 2]>::try_from(n).ok()) {
            at = line(at);
            for id in start..start + count {
                slots.insert(id, at);
                at += 20;
            }
        }
        let at = slots[&id];
        out[at..at + 10].copy_from_slice(b"0000000003");
        out
    }

    fn with_broken_object_stream(bytes: &[u8]) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        let mut out = Vec::new();
        doc.save_modern(&mut out).unwrap();
        let at = out.windows(6).position(|w| w == b"ObjStm").unwrap();
        let start = at + out[at..].windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        out[start..start + 12].iter_mut().for_each(|b| *b ^= 0xff);
        out
    }

    #[test]
    fn a_file_that_loses_objects_while_loading_is_refused() {
        let crossed = wrong_offset(&fixture(), 14);
        assert!(!Document::load_mem(&crossed).unwrap().objects.contains_key(&(14, 0)), "the lenient reader opens it and drops an object");
        assert_eq!(apply(&crossed, &PageOp::Rotate { pages: vec![0], clockwise: true }), Err(EditError::Broken));
        assert_eq!(apply(&fixture(), &PageOp::Insert { at: 0, pdf: crossed.clone() }), Err(EditError::Broken));
        assert_eq!(with_annotations(&crossed, &[], &|_| [0, 0, 0]), Err(EditError::Broken));
        let packed = with_broken_object_stream(&fixture());
        assert_eq!(apply(&packed, &PageOp::Delete(vec![0])), Err(EditError::Broken));
    }

    fn with_short_xref_entries(bytes: &[u8]) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        let at = out.windows(6).rposition(|w| w == b"\nxref\n").unwrap() + 1;
        let mut head = out[..at].to_vec();
        let rest = String::from_utf8(out[at..].to_vec()).unwrap();
        for line in rest.split_inclusive('\n') {
            match line.strip_suffix(" \n") {
                Some(entry) if entry.len() == 18 => head.extend_from_slice(format!("{entry}\n").as_bytes()),
                _ => head.extend_from_slice(line.as_bytes()),
            }
        }
        head
    }

    #[test]
    fn a_file_with_short_xref_entries_that_viewers_open_is_edited() {
        let short = with_short_xref_entries(&fixture());
        assert!(Document::load_mem_with_options(&short, LoadOptions { strict: true, ..Default::default() }).is_err(), "the strict reader refuses it");
        let out = apply(&short, &PageOp::Rotate { pages: vec![0], clockwise: true }).unwrap();
        assert_eq!(sizes(&out).len(), sizes(&fixture()).len());
        assert!(apply(&fixture(), &PageOp::Insert { at: 0, pdf: short }).is_ok());
    }

    fn with_page_entry(bytes: &[u8], key: &str, value: Object) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        let root = doc.catalog().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
        doc.get_object_mut(root).unwrap().as_dict_mut().unwrap().set(key, value);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn inserted_pages_do_not_inherit_what_sits_on_the_host_root() {
        let host = with_page_entry(&appendix(), "Rotate", 90.into());
        let host = with_page_entry(&host, "MediaBox", media(100, 100));
        let out = apply(&host, &PageOp::Insert { at: 1, pdf: fixture() }).unwrap();
        assert_eq!(sizes(&out), [(400., 300.), (792., 612.), (612., 792.), (792., 612.), (612., 792.), (400., 300.)]);
        let doc = Document::load_mem(&out).unwrap();
        let root = doc.get_dictionary(pages_root(&doc).unwrap()).unwrap();
        assert!(INHERITED.iter().all(|k| !root.has(k)), "{root:?}");
    }

    #[test]
    fn a_looping_outline_is_handled_in_a_blink() {
        let mut doc = Document::with_version("1.5");
        let root = doc.new_object_id();
        let font = font(&mut doc);
        let (one, two) = (leaf(&mut doc, root, "Eins"), leaf(&mut doc, root, "Zwei"));
        doc.objects.insert(root, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![one.into(), two.into()], "Count" => 2, "MediaBox" => media(612, 792), "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } }));
        let (outlines, a, b) = (doc.new_object_id(), doc.new_object_id(), doc.new_object_id());
        let dest = |page: ObjectId| -> Object { vec![page.into(), "Fit".into()].into() };
        doc.objects.insert(a, Object::Dictionary(dictionary! { "Title" => Object::string_literal("A"), "Parent" => outlines, "Dest" => dest(one), "Next" => b, "First" => a }));
        doc.objects.insert(b, Object::Dictionary(dictionary! { "Title" => Object::string_literal("B"), "Parent" => outlines, "Dest" => dest(two), "Prev" => a, "First" => a }));
        doc.objects.insert(outlines, Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => a, "Last" => b, "Count" => 2 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root, "Outlines" => outlines });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let started = std::time::Instant::now();
        let out = apply(&bytes, &PageOp::Delete(vec![0])).unwrap();
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
        assert_eq!(texts(&out), ["Zwei"]);
        let doc = Document::load_mem(&out).unwrap();
        let outline = doc.get_dictionary(doc.catalog().unwrap().get(b"Outlines").unwrap().as_reference().unwrap()).unwrap();
        assert_eq!(outline.get(b"Count").unwrap().as_i64().unwrap(), 1);
        assert_eq!(outline.get(b"First").unwrap(), outline.get(b"Last").unwrap());
    }

    fn widget(doc: &mut Document, page: ObjectId, parent: Option<ObjectId>, name: &str) -> ObjectId {
        let mut dict = dictionary! { "Type" => "Annot", "Subtype" => "Widget", "T" => Object::string_literal(name), "P" => page, "Rect" => vec![0.into(), 0.into(), 50.into(), 20.into()] };
        if let Some(parent) = parent {
            dict.set("Parent", parent);
        } else {
            dict.set("FT", "Tx");
        }
        let id = doc.add_object(dict);
        let dict = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
        let mut list = dict.get(b"Annots").ok().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default();
        list.push(id.into());
        dict.set("Annots", list);
        id
    }

    fn field_names(bytes: &[u8]) -> Vec<String> {
        fn names(doc: &Document, object: &Object, out: &mut Vec<String>) {
            let Some(dict) = doc.dereference(object).ok().and_then(|(_, o)| o.as_dict().ok()) else { return };
            if let Ok(t) = dict.get(b"T") {
                out.push(lopdf::decode_text_string(t).unwrap());
            }
            if let Some(kids) = dict.get(b"Kids").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_array().ok()) {
                kids.iter().for_each(|k| names(doc, k, out));
            }
        }
        let doc = Document::load_mem(bytes).unwrap();
        let form = doc.catalog().unwrap().get(b"AcroForm").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok());
        let fields = form.and_then(|f| f.get(b"Fields").ok()).and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_array().ok());
        let mut out = Vec::new();
        fields.into_iter().flatten().for_each(|f| names(&doc, f, &mut out));
        out
    }

    fn with_form(bytes: &[u8]) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        let pages: Vec<ObjectId> = doc.page_iter().collect();
        let single = widget(&mut doc, pages[0], None, "einzeln");
        let other = widget(&mut doc, pages[1], None, "bleibt");
        let group = doc.add_object(dictionary! { "FT" => "Tx", "T" => Object::string_literal("gruppe") });
        let (w1, w2) = (widget(&mut doc, pages[0], Some(group), "w1"), widget(&mut doc, pages[1], Some(group), "w2"));
        doc.get_object_mut(group).unwrap().as_dict_mut().unwrap().set("Kids", vec![Object::from(w1), w2.into()]);
        let all_gone = doc.add_object(dictionary! { "FT" => "Tx", "T" => Object::string_literal("alles weg") });
        let w3 = widget(&mut doc, pages[0], Some(all_gone), "w3");
        doc.get_object_mut(all_gone).unwrap().as_dict_mut().unwrap().set("Kids", vec![Object::from(w3)]);
        let bare = doc.add_object(dictionary! { "FT" => "Tx", "T" => Object::string_literal("ohne Widget") });
        let fields = doc.add_object(vec![Object::from(single), other.into(), group.into(), all_gone.into(), bare.into()]);
        doc.catalog_mut().unwrap().set("AcroForm", dictionary! { "Fields" => fields });
        doc.catalog_mut().unwrap().set("PageLabels", dictionary! { "Nums" => vec![0.into(), dictionary! { "S" => "r" }.into()] });
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn deleting_a_page_takes_the_form_fields_that_lived_on_it() {
        let bytes = with_form(&fixture());
        assert_eq!(field_names(&bytes), ["einzeln", "bleibt", "gruppe", "w1", "w2", "alles weg", "w3", "ohne Widget"]);
        let out = apply(&bytes, &PageOp::Delete(vec![0])).unwrap();
        assert_eq!(field_names(&out), ["bleibt", "gruppe", "w2", "ohne Widget"]);
        assert!(!contains(&out, b"alles weg") && !contains(&out, b"einzeln"));
        let out = apply(&bytes, &PageOp::Delete(vec![0, 1])).unwrap();
        assert_eq!(field_names(&out), ["ohne Widget"]);
        let out = apply(&bytes, &PageOp::Delete(vec![3])).unwrap();
        assert_eq!(field_names(&out), field_names(&bytes));
    }

    #[test]
    fn page_labels_are_dropped_when_the_page_count_or_order_changes() {
        let bytes = with_form(&fixture());
        let has_labels = |bytes: &[u8]| Document::load_mem(bytes).unwrap().catalog().unwrap().has(b"PageLabels");
        assert!(has_labels(&bytes));
        assert!(has_labels(&apply(&bytes, &PageOp::Rotate { pages: vec![0], clockwise: true }).unwrap()));
        assert!(!has_labels(&apply(&bytes, &PageOp::Delete(vec![0])).unwrap()));
        assert!(!has_labels(&apply(&bytes, &PageOp::Move { pages: vec![0], to: 2 }).unwrap()));
        assert!(!has_labels(&apply(&bytes, &PageOp::Insert { at: 0, pdf: appendix() }).unwrap()));
    }

    fn with_source_form(bytes: &[u8], da: Option<&str>) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        let page = doc.page_iter().next().unwrap();
        let field = widget(&mut doc, page, None, "quelle");
        let helv = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut form = dictionary! { "Fields" => vec![Object::from(field)], "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } } };
        if let Some(da) = da {
            form.set("DA", Object::string_literal(da));
        }
        doc.catalog_mut().unwrap().set("AcroForm", form);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn inserting_brings_the_form_fields_of_the_other_file() {
        let source = with_source_form(&appendix(), Some("/Helv 0 Tf 0 g"));
        let out = apply(&fixture(), &PageOp::Insert { at: 1, pdf: source.clone() }).unwrap();
        assert_eq!(field_names(&out), ["quelle"]);
        let doc = Document::load_mem(&out).unwrap();
        let form = doc.dereference(doc.catalog().unwrap().get(b"AcroForm").unwrap()).unwrap().1.as_dict().unwrap();
        assert_eq!(form.get(b"DA").unwrap().as_str().unwrap(), b"/Helv 0 Tf 0 g");
        let fonts = doc.dereference(form.get(b"DR").unwrap()).unwrap().1.as_dict().unwrap().get(b"Font").unwrap().as_dict().unwrap();
        assert!(fonts.has(b"Helv"));
        let annots = doc.get_page_annotations(doc.page_iter().nth(1).unwrap()).unwrap();
        assert_eq!(lopdf::decode_text_string(annots[0].get(b"T").unwrap()).unwrap(), "quelle");

        let host = with_form(&fixture());
        let out = apply(&host, &PageOp::Insert { at: 0, pdf: source }).unwrap();
        assert_eq!(field_names(&out), ["einzeln", "bleibt", "gruppe", "w1", "w2", "alles weg", "w3", "ohne Widget", "quelle"]);
        let doc = Document::load_mem(&out).unwrap();
        let form = doc.dereference(doc.catalog().unwrap().get(b"AcroForm").unwrap()).unwrap().1.as_dict().unwrap();
        assert!(form.has(b"DA") && form.has(b"DR"), "the host form takes over what the new fields need");
        let none = apply(&host, &PageOp::Insert { at: 0, pdf: appendix() }).unwrap();
        assert_eq!(field_names(&none), field_names(&host));
    }

    #[test]
    fn inserting_the_same_form_again_renames_its_fields_instead_of_doubling_them() {
        let source = with_source_form(&appendix(), None);
        let once = apply(&fixture(), &PageOp::Insert { at: 1, pdf: source.clone() }).unwrap();
        let twice = apply(&once, &PageOp::Insert { at: 0, pdf: source.clone() }).unwrap();
        let thrice = apply(&twice, &PageOp::Insert { at: 0, pdf: source }).unwrap();
        assert_eq!(field_names(&twice), ["quelle", "quelle 2"]);
        assert_eq!(field_names(&thrice), ["quelle", "quelle 2", "quelle 3"]);
        let doc = Document::load_mem(&thrice).unwrap();
        let shown: Vec<String> = doc.page_iter().flat_map(|p| doc.get_page_annotations(p).unwrap_or_default()).filter_map(|a| a.get(b"T").ok().and_then(|t| lopdf::decode_text_string(t).ok())).collect();
        assert_eq!(shown.len(), 3);
        assert!(shown.contains(&"quelle 2".to_string()) && shown.contains(&"quelle 3".to_string()), "{shown:?}");
    }

    fn with_layers(bytes: &[u8], names: &[(&str, bool)], base_off: bool) -> Vec<u8> {
        let mut doc = Document::load_mem(bytes).unwrap();
        let ids: Vec<(ObjectId, bool)> = names.iter().map(|(n, on)| (doc.add_object(dictionary! { "Type" => "OCG", "Name" => Object::string_literal(*n) }), *on)).collect();
        let mut d = Dictionary::new();
        if base_off {
            d.set("BaseState", "OFF");
            d.set("ON", ids.iter().filter(|(_, on)| *on).map(|(id, _)| Object::from(*id)).collect::<Vec<_>>());
        } else {
            d.set("OFF", ids.iter().filter(|(_, on)| !*on).map(|(id, _)| Object::from(*id)).collect::<Vec<_>>());
        }
        doc.catalog_mut().unwrap().set("OCProperties", dictionary! { "OCGs" => ids.iter().map(|(id, _)| Object::from(*id)).collect::<Vec<_>>(), "D" => d });
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn layers(bytes: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
        let doc = Document::load_mem(bytes).unwrap();
        let props = doc.dereference(doc.catalog().unwrap().get(b"OCProperties").unwrap()).unwrap().1.as_dict().unwrap();
        let named = |o: Option<&Object>| -> Vec<String> {
            let list = o.and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_array().ok()).cloned().unwrap_or_default();
            list.iter().map(|g| lopdf::decode_text_string(doc.dereference(g).unwrap().1.as_dict().unwrap().get(b"Name").unwrap()).unwrap()).collect()
        };
        let d = doc.dereference(props.get(b"D").unwrap()).unwrap().1.as_dict().unwrap();
        (named(props.get(b"OCGs").ok()), named(d.get(b"OFF").ok()), named(d.get(b"ON").ok()))
    }

    #[test]
    fn inserting_brings_the_layers_of_the_other_file_with_their_default_state() {
        let source = with_layers(&appendix(), &[("Quelle an", true), ("Quelle aus", false)], false);
        let host = with_layers(&fixture(), &[("Host", true)], false);
        let out = apply(&host, &PageOp::Insert { at: 0, pdf: source.clone() }).unwrap();
        assert_eq!(layers(&out), (vec!["Host".into(), "Quelle an".into(), "Quelle aus".into()], vec!["Quelle aus".into()], vec![]));
        let out = apply(&fixture(), &PageOp::Insert { at: 0, pdf: source }).unwrap();
        assert_eq!(layers(&out), (vec!["Quelle an".into(), "Quelle aus".into()], vec!["Quelle aus".into()], vec![]));
        let off_source = with_layers(&appendix(), &[("Quelle an", true), ("Quelle aus", false)], true);
        let out = apply(&with_layers(&fixture(), &[("Host", false)], false), &PageOp::Insert { at: 0, pdf: off_source.clone() }).unwrap();
        assert_eq!(layers(&out), (vec!["Host".into(), "Quelle an".into(), "Quelle aus".into()], vec!["Host".into(), "Quelle aus".into()], vec![]));
        let out = apply(&with_layers(&fixture(), &[("Host", true)], true), &PageOp::Insert { at: 0, pdf: off_source }).unwrap();
        assert_eq!(layers(&out).2, ["Host", "Quelle an"]);
    }
}
