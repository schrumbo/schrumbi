use std::path::Path;

use walkdir::WalkDir;

/// Every folder under `root` a module could live in, as relative paths, deepest level limited for huge trees.
pub fn all_folders(root: &Path) -> Vec<String> {
    let mut folders: Vec<String> = WalkDir::new(root)
        .min_depth(1)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.file_type().is_dir() && !name.starts_with('.') && !name.starts_with('_') && name != "node_modules"
        })
        .filter_map(Result::ok)
        .filter_map(|e| e.path().strip_prefix(root).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
        .collect();
    folders.sort_by_key(|f| f.to_lowercase());
    folders
}

fn is_semester(name: &str) -> bool {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^(ws|ss|wise|sose|sem(ester)?)[\s_-]*\d|^\d+\.?\s*sem").unwrap());
    PATTERN.is_match(name)
}

fn is_course_code(name: &str) -> bool {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[A-ZÄÖÜ]{2,6}\d{0,4}$").unwrap());
    PATTERN.is_match(name)
}

fn is_resource(name: &str) -> bool {
    const NAMES: [&str; 12] = ["assets", "attachments", "anhänge", "bilder", "images", "templates", "vorlagen", "daily", "inbox", "archiv", "archive", "privat"];
    NAMES.contains(&name.to_lowercase().as_str())
}

/// Which folders probably are modules: everything directly inside a semester folder, and anything named like a course.
/// A plain workspace without such hints starts with its top-level folders; a vault starts with nothing it cannot tell.
pub fn suggested_modules(folders: &[String], is_vault: bool) -> Vec<String> {
    let name_of = |f: &str| f.rsplit('/').next().unwrap_or(f).to_string();
    let parent_of = |f: &str| f.rsplit_once('/').map(|(p, _)| p.to_string());
    let mut chosen: Vec<String> = Vec::new();
    for folder in folders {
        let name = name_of(folder);
        if is_semester(&name) || is_resource(&name) || chosen.iter().any(|c| folder.starts_with(&format!("{c}/"))) {
            continue;
        }
        let under_semester = parent_of(folder).is_some_and(|p| is_semester(&name_of(&p)));
        if under_semester || is_course_code(&name) {
            chosen.push(folder.clone());
        }
    }
    if chosen.is_empty() && !is_vault {
        chosen = folders.iter().filter(|f| !f.contains('/') && !is_resource(f) && !is_semester(f)).cloned().collect();
    }
    chosen
}

/// The folders to show open at first, so every suggestion can be seen.
pub fn parents_of(folders: &[String]) -> Vec<String> {
    let mut open = Vec::new();
    for folder in folders {
        let mut rest = folder.as_str();
        while let Some((up, _)) = rest.rsplit_once('/') {
            open.push(up.to_string());
            rest = up;
        }
    }
    open.sort();
    open.dedup();
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_modules_at_any_depth() {
        let folders = list(&["Privat", "Privat/Rezepte", "Studium", "Studium/WS25", "Studium/WS25/Complex Analysis 1", "Studium/WS25/Complex Analysis 1/Notizen", "SS26", "SS26/Mathe 2", "MCI", "MCI/Blätter", "assets"]);
        assert_eq!(suggested_modules(&folders, true), list(&["Studium/WS25/Complex Analysis 1", "SS26/Mathe 2", "MCI"]));
    }

    #[test]
    fn a_plain_workspace_starts_with_its_top_level() {
        let folders = list(&["Analysis", "Analysis/Blatt1", "Mathe", "assets"]);
        assert_eq!(suggested_modules(&folders, false), list(&["Analysis", "Mathe"]));
        assert!(suggested_modules(&folders, true).is_empty());
    }

    #[test]
    fn opens_the_way_down_to_a_suggestion() {
        assert_eq!(parents_of(&list(&["a/b/c", "x"])), list(&["a", "a/b"]));
    }
}
