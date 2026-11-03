use std::path::{Path, PathBuf};

use serde::Deserialize;

/// How new links are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkFormat {
    #[default]
    Shortest,
    Relative,
    Absolute,
}

/// What `.obsidian/` says about a vault. Read only: Obsidian keeps these files in memory and would overwrite changes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VaultConfig {
    pub attachment_folder_path: String,
    pub new_link_format: LinkFormat,
    pub use_markdown_links: bool,
    pub strict_line_breaks: bool,
    pub always_update_links: bool,
    pub new_file_location: String,
    pub new_file_folder_path: String,
    pub user_ignore_filters: Vec<String>,
    #[serde(skip)]
    pub daily: DailyNotes,
    #[serde(skip)]
    pub templates: Templates,
    #[serde(skip)]
    pub is_vault: bool,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self { attachment_folder_path: "/".into(), new_link_format: LinkFormat::Shortest, use_markdown_links: false, strict_line_breaks: false, always_update_links: true, new_file_location: "root".into(), new_file_folder_path: String::new(), user_ignore_filters: Vec::new(), daily: DailyNotes::default(), templates: Templates::default(), is_vault: false }
    }
}

/// Settings of the daily notes plugin.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct DailyNotes {
    pub folder: String,
    pub format: String,
    pub template: String,
}

/// Settings of the templates plugin.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Templates {
    pub folder: String,
    pub date_format: String,
    pub time_format: String,
}

fn read_json<T: for<'a> Deserialize<'a> + Default>(path: PathBuf) -> T {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

impl VaultConfig {
    /// Reads the settings; a folder without `.obsidian` gets Obsidian's defaults.
    pub fn read(root: &Path) -> Self {
        let dir = root.join(".obsidian");
        let mut config: Self = read_json(dir.join("app.json"));
        config.is_vault = dir.is_dir();
        config.daily = read_json(dir.join("daily-notes.json"));
        config.templates = read_json(dir.join("templates.json"));
        config
    }

    /// Where an attachment of `note` belongs: the vault root, the note's folder, a subfolder of it, or a fixed folder.
    pub fn attachment_folder(&self, root: &Path, note: &Path) -> PathBuf {
        let setting = self.attachment_folder_path.trim();
        let beside = note.parent().unwrap_or(root);
        if setting.is_empty() || setting == "/" {
            root.to_path_buf()
        } else if setting == "." || setting == "./" {
            beside.to_path_buf()
        } else if let Some(sub) = setting.strip_prefix("./") {
            beside.join(sub)
        } else {
            root.join(setting.trim_start_matches('/'))
        }
    }

    /// Where a new note without a given folder goes.
    pub fn new_note_folder(&self, root: &Path, current: Option<&Path>) -> PathBuf {
        match self.new_file_location.as_str() {
            "current" => current.and_then(Path::parent).unwrap_or(root).to_path_buf(),
            "folder" if !self.new_file_folder_path.is_empty() => root.join(self.new_file_folder_path.trim_start_matches('/')),
            _ => root.to_path_buf(),
        }
    }

    /// Whether Obsidian's "excluded files" list hides this path.
    pub fn is_ignored(&self, relative: &str) -> bool {
        self.user_ignore_filters.iter().any(|filter| {
            if let Some(pattern) = filter.strip_prefix('/').and_then(|f| f.strip_suffix('/')) {
                regex::Regex::new(pattern).is_ok_and(|re| re.is_match(relative))
            } else {
                relative.starts_with(filter.as_str())
            }
        })
    }

    pub fn plugin_enabled(root: &Path, id: &str) -> bool {
        std::fs::read_to_string(root.join(".obsidian/community-plugins.json")).is_ok_and(|s| s.contains(&format!("\"{id}\"")))
    }
}

/// Settings of the unique note creator.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UniqueNotes {
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub template: String,
}

/// Reads `.obsidian/zk-prefixer.json`.
pub fn unique_notes(root: &Path) -> UniqueNotes {
    read_json(root.join(".obsidian/zk-prefixer.json"))
}

/// The property types in `.obsidian/types.json`.
pub fn property_types(root: &Path) -> std::collections::HashMap<String, super::frontmatter::PropertyKind> {
    use super::frontmatter::PropertyKind as K;
    let Ok(bytes) = std::fs::read(root.join(".obsidian/types.json")) else { return Default::default() };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return Default::default() };
    let Some(types) = value.get("types").and_then(|v| v.as_object()) else { return Default::default() };
    types
        .iter()
        .filter_map(|(name, kind)| {
            let kind = match kind.as_str()? {
                "text" => K::Text,
                "multitext" => K::List,
                "number" => K::Number,
                "checkbox" => K::Checkbox,
                "date" => K::Date,
                "datetime" => K::DateTime,
                "tags" => K::Tags,
                _ => return None,
            };
            Some((name.clone(), kind))
        })
        .collect()
}

/// Whether Obsidian can be asked to open files of this vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Known,
    /// Obsidian is here, but this folder has never been opened as a vault.
    UnknownVault,
    NotInstalled,
}

/// Decides from Obsidian's own list of vaults.
pub fn reach(root: &Path, known: &[KnownVault], installed: bool) -> Reach {
    if known.iter().any(|v| root.starts_with(&v.path) || dunce::canonicalize(&v.path).is_ok_and(|p| root.starts_with(p))) {
        Reach::Known
    } else if installed || !known.is_empty() {
        Reach::UnknownVault
    } else {
        Reach::NotInstalled
    }
}

/// Whether Obsidian seems to be installed on this device.
pub fn obsidian_installed() -> bool {
    obsidian_dir().is_some() || Path::new("/Applications/Obsidian.app").exists() || dirs::home_dir().is_some_and(|h| h.join("Applications/Obsidian.app").exists())
}

/// A bookmark from Obsidian's own list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bookmark {
    pub title: String,
    pub path: Option<String>,
    pub subpath: Option<String>,
    pub query: Option<String>,
}

fn collect_bookmarks(items: &[serde_json::Value], out: &mut Vec<Bookmark>) {
    for item in items {
        let text = |k: &str| item.get(k).and_then(|v| v.as_str()).map(String::from);
        match item.get("type").and_then(|v| v.as_str()) {
            Some("group") => collect_bookmarks(item.get("items").and_then(|v| v.as_array()).map_or(&[][..], Vec::as_slice), out),
            Some("search") => out.push(Bookmark { title: text("title").or(text("query")).unwrap_or_default(), query: text("query"), ..Default::default() }),
            Some(_) => {
                let path = text("path");
                let name = path.as_deref().map(|p| p.rsplit('/').next().unwrap_or(p).trim_end_matches(".md").to_string());
                out.push(Bookmark { title: text("title").filter(|t| !t.is_empty()).or(name).unwrap_or_default(), path, subpath: text("subpath"), query: None });
            }
            None => {}
        }
    }
}

/// The vault's bookmarks, flat; groups only order them.
pub fn bookmarks(root: &Path) -> Vec<Bookmark> {
    let Ok(bytes) = std::fs::read(root.join(".obsidian/bookmarks.json")) else { return Vec::new() };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return Vec::new() };
    let mut out = Vec::new();
    collect_bookmarks(value.get("items").and_then(|v| v.as_array()).map_or(&[][..], Vec::as_slice), &mut out);
    out
}

/// Whether a file of the vault has a bookmark.
pub fn is_bookmarked(root: &Path, rel: &str) -> bool {
    bookmarks(root).iter().any(|b| b.path.as_deref() == Some(rel))
}

/// Adds or removes a file's bookmark in the file Obsidian uses; other bookmarks and groups stay as they are.
pub fn toggle_bookmark(root: &Path, rel: &str) -> std::io::Result<()> {
    let path = root.join(".obsidian/bookmarks.json");
    let mut value: serde_json::Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({ "items": [] }));
    if !value.get("items").is_some_and(serde_json::Value::is_array) {
        value["items"] = serde_json::json!([]);
    }
    fn remove(items: &mut Vec<serde_json::Value>, rel: &str) -> bool {
        let before = items.len();
        items.retain(|i| !(i.get("type").and_then(|t| t.as_str()) == Some("file") && i.get("path").and_then(|p| p.as_str()) == Some(rel) && i.get("subpath").is_none()));
        let mut found = items.len() != before;
        for item in items.iter_mut() {
            if let Some(inner) = item.get_mut("items").and_then(|v| v.as_array_mut()) {
                found |= remove(inner, rel);
            }
        }
        found
    }
    let items = value["items"].as_array_mut().expect("checked above");
    if !remove(items, rel) {
        items.push(serde_json::json!({ "type": "file", "ctime": chrono::Utc::now().timestamp_millis(), "path": rel }));
    }
    std::fs::create_dir_all(root.join(".obsidian"))?;
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap_or_default())
}

/// A vault Obsidian knows on this device.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownVault {
    pub name: String,
    pub path: PathBuf,
}

/// The vaults listed in Obsidian's own settings, for choosing one. Never written.
pub fn known_vaults() -> Vec<KnownVault> {
    let Some(dir) = obsidian_dir() else { return Vec::new() };
    let Ok(bytes) = std::fs::read(dir.join("obsidian.json")) else { return Vec::new() };
    parse_known(&bytes)
}

fn obsidian_dir() -> Option<PathBuf> {
    if crate::core::drive::on() {
        return None;
    }
    let base = dirs::config_dir()?;
    [base.join("obsidian"), base.join("Obsidian")].into_iter().find(|d| d.is_dir())
}

pub fn parse_known(bytes: &[u8]) -> Vec<KnownVault> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else { return Vec::new() };
    let Some(vaults) = value.get("vaults").and_then(|v| v.as_object()) else { return Vec::new() };
    let mut list: Vec<KnownVault> = vaults
        .values()
        .filter_map(|v| v.get("path").and_then(|p| p.as_str()))
        .map(PathBuf::from)
        .map(|path| KnownVault { name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), path })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// What an `obsidian://` address asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum UriAction {
    Open(String),
    New { name: String, content: String },
    Daily,
    Search(String),
}

fn decode(value: &str) -> String {
    crate::core::markdown::percent_decode(value)
}

/// Reads an `obsidian://` address when it points at the vault in `root`; others are left to the system.
pub fn parse_uri(uri: &str, root: &Path) -> Option<UriAction> {
    let rest = uri.strip_prefix("obsidian://")?;
    let name = root.file_name()?.to_string_lossy().to_string();
    if let Some(short) = rest.strip_prefix("vault/") {
        let (vault, file) = short.split_once('/').unwrap_or((short, ""));
        return (decode(vault) == name && !file.is_empty()).then(|| UriAction::Open(decode(file)));
    }
    if rest.starts_with('/') {
        return decode(rest).strip_prefix(&format!("{}/", root.display())).map(|f| UriAction::Open(f.to_string()));
    }
    let (kind, query) = rest.split_once('?').unwrap_or((rest, ""));
    let get = |key: &str| query.split('&').find_map(|p| p.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| decode(v)));
    if let Some(path) = get("path") {
        return path.strip_prefix(&format!("{}/", root.display())).map(|f| UriAction::Open(f.to_string()));
    }
    if get("vault").is_some_and(|v| v != name) {
        return None;
    }
    match kind.trim_end_matches('/') {
        "open" => get("file").map(UriAction::Open),
        "new" => Some(UriAction::New { name: get("name").or(get("file")).unwrap_or_default(), content: get("content").unwrap_or_default() }),
        "daily" => Some(UriAction::Daily),
        "search" => Some(UriAction::Search(get("query").unwrap_or_default())),
        _ => None,
    }
}

/// An `obsidian://` address that opens a file in the vault containing it.
pub fn open_uri(path: &Path) -> String {
    let encoded: String = path.to_string_lossy().bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("obsidian://open?path={encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_fall_back_to_obsidians_defaults() {
        let dir = std::env::temp_dir().join(format!("schrumbi-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
        let empty = VaultConfig::read(&dir);
        assert!(empty.is_vault);
        assert!(!empty.use_markdown_links);
        assert!(!empty.strict_line_breaks);
        std::fs::write(dir.join(".obsidian/app.json"), r#"{"attachmentFolderPath":"./anhang","newLinkFormat":"relative","useMarkdownLinks":true,"strictLineBreaks":true,"userIgnoreFilters":["Archiv/"],"unbekannt":1}"#).unwrap();
        std::fs::write(dir.join(".obsidian/daily-notes.json"), r#"{"folder":"Tage","format":"YYYY-MM-DD","template":"V/Tag"}"#).unwrap();
        std::fs::write(dir.join(".obsidian/templates.json"), r#"{"folder":"V","dateFormat":"DD.MM."}"#).unwrap();
        let c = VaultConfig::read(&dir);
        assert_eq!(c.new_link_format, LinkFormat::Relative);
        assert!(c.use_markdown_links && c.strict_line_breaks);
        assert_eq!(c.daily.folder, "Tage");
        assert_eq!(c.templates.date_format, "DD.MM.");
        assert!(c.is_ignored("Archiv/alt.md"));
        assert!(!c.is_ignored("Anderes/alt.md"));
        assert_eq!(c.attachment_folder(&dir, &dir.join("a/n.md")), dir.join("a/anhang"));
        assert!(!VaultConfig::read(&dir.join("gibt-es-nicht")).is_vault);
    }

    #[test]
    fn attachment_settings_cover_all_four_forms() {
        let root = Path::new("/v");
        let note = Path::new("/v/a/n.md");
        let with = |s: &str| VaultConfig { attachment_folder_path: s.into(), ..Default::default() }.attachment_folder(root, note);
        assert_eq!(with("/"), PathBuf::from("/v"));
        assert_eq!(with("./"), PathBuf::from("/v/a"));
        assert_eq!(with("./bilder"), PathBuf::from("/v/a/bilder"));
        assert_eq!(with("Anhang"), PathBuf::from("/v/Anhang"));
    }

    #[test]
    fn bookmarks_toggle_and_keep_the_others() {
        let dir = std::env::temp_dir().join(format!("schrumbi-marks-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
        std::fs::write(dir.join(".obsidian/bookmarks.json"), r#"{"items":[{"type":"group","title":"G","items":[{"type":"file","path":"a.md"}]},{"type":"search","query":"x"}]}"#).unwrap();
        toggle_bookmark(&dir, "b.md").unwrap();
        assert!(is_bookmarked(&dir, "b.md") && is_bookmarked(&dir, "a.md"));
        toggle_bookmark(&dir, "a.md").unwrap();
        assert!(!is_bookmarked(&dir, "a.md"));
        assert!(bookmarks(&dir).iter().any(|b| b.query.is_some()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bookmarks_read_all_kinds_flat() {
        let dir = std::env::temp_dir().join(format!("schrumbi-bookmarks-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
        std::fs::write(dir.join(".obsidian/bookmarks.json"), r##"{"items":[{"type":"file","path":"a/N.md"},{"type":"group","title":"G","items":[{"type":"heading","path":"b.md","subpath":"#H","title":"Titel"},{"type":"search","query":"tag:x"}]},{"type":"folder","path":"a"}]}"##).unwrap();
        let b = bookmarks(&dir);
        assert_eq!(b.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["N", "Titel", "tag:x", "a"]);
        assert_eq!(b[1].subpath.as_deref(), Some("#H"));
        assert_eq!(b[2].query.as_deref(), Some("tag:x"));
        assert!(bookmarks(&dir.join("nichts")).is_empty());
    }

    #[test]
    fn knows_whether_obsidian_can_open_a_vault() {
        let known = vec![KnownVault { name: "V".into(), path: PathBuf::from("/x/V") }];
        assert_eq!(reach(Path::new("/x/V"), &known, true), Reach::Known);
        assert_eq!(reach(Path::new("/x/Anders"), &known, true), Reach::UnknownVault);
        assert_eq!(reach(Path::new("/x/V"), &[], true), Reach::UnknownVault);
        assert_eq!(reach(Path::new("/x/V"), &[], false), Reach::NotInstalled);
    }

    #[test]
    fn addresses_to_this_vault_stay_here() {
        let root = Path::new("/x/Mein Vault");
        assert_eq!(parse_uri("obsidian://open?vault=Mein%20Vault&file=Ordner%2FNote", root), Some(UriAction::Open("Ordner/Note".into())));
        assert_eq!(parse_uri("obsidian://open?vault=Anderer&file=Note", root), None);
        assert_eq!(parse_uri("obsidian://open?path=%2Fx%2FMein%20Vault%2FA.md", root), Some(UriAction::Open("A.md".into())));
        assert_eq!(parse_uri("obsidian://open?path=%2Fy%2FA.md", root), None);
        assert_eq!(parse_uri("obsidian://vault/Mein%20Vault/A", root), Some(UriAction::Open("A".into())));
        assert_eq!(parse_uri("obsidian:///x/Mein%20Vault/A.md", root), Some(UriAction::Open("A.md".into())));
        assert_eq!(parse_uri("obsidian://new?name=N&content=T", root), Some(UriAction::New { name: "N".into(), content: "T".into() }));
        assert_eq!(parse_uri("obsidian://daily?vault=Mein%20Vault", root), Some(UriAction::Daily));
        assert_eq!(parse_uri("obsidian://search?query=tag%3Ax", root), Some(UriAction::Search("tag:x".into())));
        assert_eq!(parse_uri("https://a.b", root), None);
    }

    #[test]
    fn known_vaults_and_uris() {
        let list = parse_known(br#"{"vaults":{"a":{"path":"/x/Zweit","ts":1},"b":{"path":"/x/Erst","ts":2,"open":true}}}"#);
        assert_eq!(list.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["Erst", "Zweit"]);
        assert!(parse_known(b"kaputt").is_empty());
        assert_eq!(open_uri(Path::new("/a b/Ü.md")), "obsidian://open?path=%2Fa%20b%2F%C3%9C.md");
    }
}
