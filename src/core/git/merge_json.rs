use serde_json::{Map, Value};

use crate::core::json::to_swift_json;
use crate::core::model::{StudyConfig, StudyState};
use crate::core::workspace::{CONFIG_NAME, STATE_NAME};

/// Merges the app's own JSON files three-way; other files return `None`, as does a real contradiction.
pub fn merge_file(path: &str, base: Option<&str>, ours: &str, theirs: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if !path_is_root(path) {
        return None;
    }
    if name == crate::core::cards::STORE_NAME {
        return crate::core::cards::merge_store(ours, theirs);
    }
    if name == crate::core::cards::LOG_NAME {
        return Some(crate::core::cards::merge_log(ours, theirs));
    }
    if name == CONFIG_NAME {
        let merged = merge_values(base, ours, theirs, Policy::Config)?;
        let typed: StudyConfig = serde_json::from_value(merged).ok()?;
        to_swift_json(&typed).ok()
    } else if name == crate::core::highlights::FILE_NAME || name == crate::core::highlights::ANNOTATIONS_NAME {
        let merged = merge_values(base, ours, theirs, Policy::State)?;
        to_swift_json(&merged).ok()
    } else if name == STATE_NAME {
        let merged = merge_values(base, ours, theirs, Policy::State)?;
        let typed: StudyState = serde_json::from_value(merged).ok()?;
        to_swift_json(&typed).ok()
    } else {
        None
    }
}

fn path_is_root(path: &str) -> bool {
    !path.contains('/')
}

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    Config,
    State,
}

fn merge_values(base: Option<&str>, ours: &str, theirs: &str, policy: Policy) -> Option<Value> {
    let parse = |s: &str| serde_json::from_str::<Value>(s).ok();
    let base = base.and_then(parse).unwrap_or(Value::Null);
    merge(&base, &parse(ours)?, &parse(theirs)?, policy, &[])
}

/// One side's change wins over no change, the same change counts once, and only two different changes are a problem.
fn merge(base: &Value, ours: &Value, theirs: &Value, policy: Policy, path: &[&str]) -> Option<Value> {
    if ours == theirs || theirs == base {
        return Some(ours.clone());
    }
    if ours == base {
        return Some(theirs.clone());
    }
    match (ours, theirs) {
        (Value::Object(o), Value::Object(t)) => merge_objects(base.as_object(), o, t, policy, path),
        (Value::Array(o), Value::Array(t)) if has_ids(o) && has_ids(t) => merge_lists(base.as_array(), o, t, policy, path),
        (Value::Array(o), Value::Array(t)) if policy == Policy::State && path.first() == Some(&"linkedFiles") => Some(merge_sets(base.as_array(), o, t)),
        _ => contradiction(ours, theirs, policy, path),
    }
}

fn merge_objects(base: Option<&Map<String, Value>>, ours: &Map<String, Value>, theirs: &Map<String, Value>, policy: Policy, path: &[&str]) -> Option<Value> {
    let mut out = Map::new();
    let null = Value::Null;
    let mut keys: Vec<&String> = ours.keys().collect();
    keys.extend(theirs.keys().filter(|k| !ours.contains_key(*k)));
    for key in keys {
        let b = base.and_then(|m| m.get(key)).unwrap_or(&null);
        let (o, t) = (ours.get(key).unwrap_or(&null), theirs.get(key).unwrap_or(&null));
        let mut inner: Vec<&str> = path.to_vec();
        inner.push(key);
        let merged = merge(b, o, t, policy, &inner)?;
        if !merged.is_null() || (ours.contains_key(key) && theirs.contains_key(key)) {
            out.insert(key.clone(), merged);
        }
    }
    Some(Value::Object(out))
}

fn has_ids(list: &[Value]) -> bool {
    list.iter().all(|v| v.get("id").is_some())
}

fn merge_lists(base: Option<&Vec<Value>>, ours: &[Value], theirs: &[Value], policy: Policy, path: &[&str]) -> Option<Value> {
    let find = |list: &[Value], id: &Value| list.iter().find(|v| v.get("id") == Some(id)).cloned();
    let empty = Vec::new();
    let base = base.unwrap_or(&empty);
    let null = Value::Null;
    let mut out = Vec::new();
    for item in ours.iter().chain(theirs.iter().filter(|t| find(ours, &t["id"]).is_none())) {
        let id = &item["id"];
        let (b, o, t) = (find(base, id), find(ours, id), find(theirs, id));
        let merged = merge(b.as_ref().unwrap_or(&null), o.as_ref().unwrap_or(&null), t.as_ref().unwrap_or(&null), policy, path)?;
        if !merged.is_null() {
            out.push(merged);
        }
    }
    Some(Value::Array(out))
}

/// Both sides' additions stay, and what either side took out of the base stays out.
fn merge_sets(base: Option<&Vec<Value>>, ours: &[Value], theirs: &[Value]) -> Value {
    let empty = Vec::new();
    let base = base.unwrap_or(&empty);
    let kept = |v: &&Value, other: &[Value]| !base.contains(v) || other.contains(v);
    let mut out: Vec<Value> = ours.iter().filter(|v| kept(v, theirs)).cloned().collect();
    out.extend(theirs.iter().filter(|v| !ours.contains(v) && !base.contains(v)).cloned());
    Value::Array(out)
}

/// The state holds marks made over time: the later one counts, and two notes both stay.
fn contradiction(ours: &Value, theirs: &Value, policy: Policy, path: &[&str]) -> Option<Value> {
    if policy == Policy::Config {
        return None;
    }
    match path.first().copied() {
        Some("notes") => match (ours.as_str(), theirs.as_str()) {
            (Some(o), Some(t)) => Some(Value::String(format!("{o}\n\n{t}"))),
            _ => Some(if ours.is_null() { theirs.clone() } else { ours.clone() }),
        },
        Some("moved") => Some(later(ours.as_str(), theirs.as_str(), ours, theirs)),
        Some("overrides") => Some(later(ours.get("date").and_then(Value::as_str).or(ours.as_str()), theirs.get("date").and_then(Value::as_str).or(theirs.as_str()), ours, theirs)),
        _ => Some(if ours.is_null() { theirs.clone() } else { ours.clone() }),
    }
}

/// ISO 8601 text sorts by time.
fn later(a: Option<&str>, b: Option<&str>, ours: &Value, theirs: &Value) -> Value {
    if b > a { theirs.clone() } else { ours.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(v: Value) -> String {
        v.to_string()
    }

    #[test]
    fn links_added_on_both_machines_all_stay() {
        let base = state(json!({"linkedFiles": {"e": ["a.pdf", "b.pdf"]}}));
        let ours = state(json!({"linkedFiles": {"e": ["a.pdf", "b.pdf", "mac.pdf"]}}));
        let theirs = state(json!({"linkedFiles": {"e": ["a.pdf", "win.md"]}}));
        let s: StudyState = serde_json::from_str(&merge_file(STATE_NAME, Some(&base), &ours, &theirs).unwrap()).unwrap();
        assert_eq!(s.linked_files["e"], vec!["a.pdf", "mac.pdf", "win.md"]);
    }

    #[test]
    fn independent_changes_both_stay() {
        let base = state(json!({"overrides": {}, "notes": {"a": "x"}}));
        let ours = state(json!({"overrides": {"e1": {"status": "done", "date": "2026-10-01T10:00:00Z"}}, "notes": {"a": "x"}}));
        let theirs = state(json!({"overrides": {}, "notes": {"a": "x", "b": "neu"}}));
        let merged = merge_file(STATE_NAME, Some(&base), &ours, &theirs).unwrap();
        let s: StudyState = serde_json::from_str(&merged).unwrap();
        assert!(s.overrides.contains_key("e1"));
        assert_eq!(s.notes["b"], "neu");
    }

    #[test]
    fn the_later_mark_wins_and_both_notes_stay() {
        let base = state(json!({"notes": {"a": "alt"}}));
        let ours = state(json!({"overrides": {"e": {"status": "done", "date": "2026-10-01T10:00:00Z"}}, "notes": {"a": "meins"}}));
        let theirs = state(json!({"overrides": {"e": {"status": "skipped", "date": "2026-10-02T10:00:00Z"}}, "notes": {"a": "fremd"}}));
        let s: StudyState = serde_json::from_str(&merge_file(STATE_NAME, Some(&base), &ours, &theirs).unwrap()).unwrap();
        assert_eq!(s.overrides["e"].status, crate::core::model::Status::Skipped);
        assert_eq!(s.notes["a"], "meins\n\nfremd");
    }

    #[test]
    fn deleting_on_one_side_deletes() {
        let base = state(json!({"notes": {"a": "x", "b": "y"}}));
        let ours = state(json!({"notes": {"b": "y"}}));
        let theirs = state(json!({"notes": {"a": "x", "b": "y", "c": "z"}}));
        let s: StudyState = serde_json::from_str(&merge_file(STATE_NAME, Some(&base), &ours, &theirs).unwrap()).unwrap();
        assert!(!s.notes.contains_key("a"));
        assert!(s.notes.contains_key("c"));
    }

    fn config(name: &str, modules: Value) -> String {
        json!({"version": 2, "name": name, "author": "M", "inbox": "_Inbox", "semesters": [], "modules": modules}).to_string()
    }

    #[test]
    fn config_lists_merge_by_id() {
        let m = |id: &str, name: &str| json!({"id": id, "code": id, "name": name, "semester": "s", "folder": id, "color": "blue"});
        let base = config("S", json!([m("a", "A")]));
        let ours = config("S", json!([m("a", "A neu"), m("b", "B")]));
        let theirs = config("S", json!([m("a", "A"), m("c", "C")]));
        let c: StudyConfig = serde_json::from_str(&merge_file(CONFIG_NAME, Some(&base), &ours, &theirs).unwrap()).unwrap();
        let ids: Vec<&str> = c.modules.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(c.modules[0].name, "A neu");
    }

    #[test]
    fn a_real_contradiction_in_the_config_goes_to_the_user() {
        let (base, ours, theirs) = (config("S", json!([])), config("Meins", json!([])), config("Fremd", json!([])));
        assert_eq!(merge_file(CONFIG_NAME, Some(&base), &ours, &theirs), None);
    }

    #[test]
    fn highlights_from_two_devices_both_stay() {
        let name = crate::core::highlights::FILE_NAME;
        let item = |id: u64| json!({"id": id, "color": "yellow", "text": "t", "parts": []});
        let base = json!({"k": []}).to_string();
        let ours = json!({"k": [item(1)]}).to_string();
        let theirs = json!({"k": [item(2)], "l": [item(3)]}).to_string();
        let merged: Value = serde_json::from_str(&merge_file(name, Some(&base), &ours, &theirs).unwrap()).unwrap();
        assert_eq!(merged["k"].as_array().unwrap().len(), 2);
        assert_eq!(merged["l"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn annotations_from_two_devices_both_stay() {
        let name = crate::core::highlights::ANNOTATIONS_NAME;
        let note = |text: &str| json!({"note": text, "text_fingerprint": "f"});
        let base = json!({"version": 1, "documents": {}}).to_string();
        let ours = json!({"version": 1, "documents": {"k": {"notes": {"1": note("a")}, "items": []}}}).to_string();
        let theirs = json!({"version": 1, "documents": {"k": {"notes": {"2": note("b")}, "items": []}}}).to_string();
        let merged: Value = serde_json::from_str(&merge_file(name, Some(&base), &ours, &theirs).unwrap()).unwrap();
        assert_eq!(merged["documents"]["k"]["notes"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn other_files_and_broken_json_are_left_alone() {
        assert_eq!(merge_file("notiz.md", None, "a", "b"), None);
        assert_eq!(merge_file("Ordner/schrumbi.json", None, "{}", "{}"), None);
        assert_eq!(merge_file(STATE_NAME, None, "kaputt", "{}"), None);
    }
}
