use chrono::{DateTime, Local};

use super::status::ChangedFile;
use crate::core::i18n::{tr, trf};

/// The text of an automatic state: when, which modules and how much.
pub fn auto_message(files: &[ChangedFile], modules: &[(String, String)], now: DateTime<Local>) -> String {
    let mut codes: Vec<&str> = Vec::new();
    for file in files {
        if let Some((_, code)) = modules.iter().filter(|(folder, _)| file.path.starts_with(&format!("{folder}/"))).max_by_key(|(folder, _)| folder.len())
            && !codes.contains(&code.as_str())
        {
            codes.push(code);
        }
    }
    let count = if files.len() == 1 { tr("1 Datei") } else { trf("%@ Dateien", &[&files.len().to_string()]) };
    let when = now.format("%d.%m.%Y, %H:%M");
    let stand = tr("Stand");
    if codes.is_empty() { format!("{stand} {when} ({count})") } else { format!("{stand} {when}: {} ({count})", codes.join(", ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::git::status::ChangeKind;
    use chrono::TimeZone;

    fn file(path: &str) -> ChangedFile {
        ChangedFile { path: path.into(), kind: ChangeKind::Modified }
    }

    #[test]
    fn names_modules_and_counts_files() {
        let now = Local.with_ymd_and_hms(2026, 10, 31, 16, 52, 0).unwrap();
        let modules = vec![("P1".to_string(), "P1".to_string()), ("P1/Sub".to_string(), "SUB".to_string()), ("SuM".to_string(), "SuM".to_string())];
        let m = auto_message(&[file("P1/a.md"), file("SuM/b.pdf"), file("P1/c.md"), file("P1/Sub/d.md")], &modules, now);
        assert!(m.contains("31.10.2026, 16:52: P1, SuM, SUB ("), "{m}");
        assert!(auto_message(&[file("frei.md")], &modules, now).ends_with("(1 Datei)") || auto_message(&[file("frei.md")], &modules, now).ends_with("(1 file)"));
    }
}
