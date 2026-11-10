use std::collections::BTreeSet;

/// What a tip is only worth while the workspace has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    Nothing,
    Modules,
    Deadlines,
}

/// One tip of the day. Texts are German source for `tr()`; `{key:…}` names a shortcut as the docs do.
#[derive(Debug, Clone, Copy)]
pub struct Tip {
    pub id: &'static str,
    /// The feature it teaches; a used feature retires its tip.
    pub feature: &'static str,
    pub icon: &'static str,
    pub title: &'static str,
    pub text: &'static str,
    pub needs: Needs,
}

const fn tip(id: &'static str, feature: &'static str, icon: &'static str, title: &'static str, text: &'static str, needs: Needs) -> Tip {
    Tip { id, feature, icon, title, text, needs }
}

/// Every tip, most useful first.
pub const TIPS: [Tip; 20] = [
    tip("quick-open", "quick_open", "search", "Jede Datei in zwei Sekunden", "{key:quickOpen} öffnet eine Suche nach Dateinamen. Ein paar Buchstaben tippen, Enter, fertig.", Needs::Nothing),
    tip("palette", "palette", "command", "Jeder Befehl per Suche", "{key:CommandPalette} zeigt alles, was die App kann. Tippe ein Wort wie „PDF“ oder „Modul“ und wähle den Befehl.", Needs::Nothing),
    tip("drop-filing", "filing", "inbox", "Dateien per Ziehen ablegen", "Zieh eine Datei auf ein Modul in der Seitenleiste oder auf eine Abgabe. Die App schlägt Ort und Namen vor, Enter legt ab.", Needs::Modules),
    tip("status-circle", "status", "circle-check", "Abhaken mit dem Kreis", "Ein Klick auf den Kreis vor einem Eintrag setzt ihn auf Erledigt oder Hochgeladen. Noch ein Klick gibt die automatische Erkennung zurück.", Needs::Deadlines),
    tip("undo", "undo", "undo-2", "Alles lässt sich zurücknehmen", "{key:Undo} nimmt Ablegen, Umbenennen, Löschen und Statuswechsel zurück, auch mehrere Schritte nacheinander.", Needs::Nothing),
    tip("cards-syntax", "cards", "layers", "Karten direkt in der Notiz", "Schreibe „Frage :: Antwort“ in eine Zeile deiner Notiz. Die Karte erscheint unter Karten und wird mit Abstand wiederholt.", Needs::Nothing),
    tip("new-entry", "new_entry", "calendar-plus", "Schnell eintragen", "{key:newEntry} trägt eine Abgabe, Aufgabe oder Prüfung ein. Das Datum darf „Fr“, „morgen“ oder „in 2 Wochen“ heißen.", Needs::Nothing),
    tip("daily-note", "daily_note", "calendar", "Die Notiz von heute", "{key:dailyNote} öffnet die Tagesnotiz, für Mitschriften und alles, was gerade anfällt.", Needs::Nothing),
    tip("split-view", "split", "columns-2", "PDF neben der Notiz", "Rechtsklick auf einen Tab, dann „Daneben öffnen“: Aufgabenblatt links, deine Lösung rechts.", Needs::Nothing),
    tip("pdf-embed", "pdf_embed", "file-text", "Eine PDF-Seite in der Notiz", "Schreibe ![[Skript.pdf#page=3]] in eine Notiz, und Seite 3 steht direkt im Text. Ein Klick öffnet die PDF dort.", Needs::Nothing),
    tip("search-slash", "search", "search", "Suchen mit einer Taste", "Die Taste / springt ins Suchfeld. Gesucht wird in Dateinamen, PDFs und Notizen, jeder Treffer führt an seine Stelle.", Needs::Nothing),
    tip("wikilink", "wikilink", "link", "Notizen verbinden", "Tippe [[ und den Namen einer Notiz, um sie zu verlinken. Rückverweise stehen rechts in den Details.", Needs::Nothing),
    tip("math", "math", "sigma", "Formeln wie in LaTeX", "$x^2$ in einer Notiz wird zur Formel, $$ … $$ bekommt eine eigene Zeile. Der PDF-Export setzt sie sauber.", Needs::Nothing),
    tip("tasks", "tasks", "square-check", "Aufgaben in Notizen", "- [ ] vor einer Zeile macht eine Aufgabe daraus. Unter Notizen → Aufgaben siehst du alle offenen auf einen Blick.", Needs::Nothing),
    tip("export-pdf", "export_pdf", "share", "Notiz als PDF abgeben", "{key:exportPDF} macht aus der offenen Notiz ein sauber gesetztes PDF, Formeln und Bilder inklusive.", Needs::Nothing),
    tip("module-keys", "module_keys", "book-open", "Module per Tastatur", "{key:goModule1}, {key:goModule2} und {key:goModule3} springen zu den ersten drei Modulen der Seitenleiste.", Needs::Modules),
    tip("reschedule", "reschedule", "calendar-clock", "Termin verschoben?", "Rechtsklick auf eine Abgabe, dann „Termin verschieben…“. Nur dieser eine Termin wandert, die Regel bleibt.", Needs::Deadlines),
    tip("day-entry", "day_entry", "calendar-plus", "Für einen Tag eintragen", "Rechtsklick auf einen Tag unter „Nächste 7 Tage“ trägt dort eine Abgabe, Aufgabe oder Prüfung ein. Das Datum steht schon drin.", Needs::Nothing),
    tip("rename", "rename", "pencil", "Umbenennen ohne Nebenwirkung", "{key:renameNote} benennt die offene Notiz oder die gewählte Datei um. Links aus anderen Notizen ziehen mit.", Needs::Nothing),
    tip("shortcuts", "shortcuts", "keyboard", "Alle Kürzel auf einen Blick", "{key:ShowShortcuts} zeigt jede Taste der App. In den Einstellungen änderst du fast alle davon.", Needs::Nothing),
];

/// The next tip to show: an unseen one about something not yet used first, then any unseen one,
/// then the one seen longest ago. A used feature never comes up again.
pub fn pick<'a>(available: &[&'a Tip], used: &BTreeSet<String>, seen: &[String]) -> Option<&'a Tip> {
    let fresh: Vec<&Tip> = available.iter().copied().filter(|t| !used.contains(t.feature)).collect();
    if let Some(t) = fresh.iter().find(|t| !seen.contains(&t.id.to_string())) {
        return Some(t);
    }
    seen.iter().find_map(|id| fresh.iter().find(|t| t.id == id).copied())
}

/// Remembers that a tip was shown, newest last and only once.
pub fn remember(seen: &mut Vec<String>, id: &str) {
    seen.retain(|s| s != id);
    seen.push(id.to_string());
}

/// The `{key:…}` names in a tip text.
pub fn key_names(text: &str) -> Vec<String> {
    super::docs::placeholders(text).into_iter().filter(|(kind, _)| kind == "key").map(|(_, name)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_id(id: &str) -> &'static Tip {
        TIPS.iter().find(|t| t.id == id).expect("tip exists")
    }

    #[test]
    fn ids_are_unique_and_keys_resolve() {
        let mut ids: Vec<&str> = TIPS.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TIPS.len());
        let actions = ["CommandPalette", "Undo", "ShowShortcuts"];
        for t in &TIPS {
            for name in key_names(t.text) {
                let known = crate::core::shortcuts::ShortcutAction::ALL.iter().any(|a| a.id() == name) || actions.contains(&name.as_str()) || crate::core::docs::plain_key(&name).is_some();
                assert!(known, "{}: unknown key {name}", t.id);
            }
        }
    }

    #[test]
    fn every_tip_reads_in_english() {
        let catalog: std::collections::HashMap<String, serde_json::Value> = serde_json::from_str(include_str!("../../assets/i18n/en.json")).unwrap();
        for t in &TIPS {
            for s in [t.title, t.text] {
                assert!(catalog.contains_key(s), "{}: {s}", t.id);
            }
        }
    }

    #[test]
    fn unseen_tips_about_unused_features_come_first() {
        let all: Vec<&Tip> = TIPS.iter().collect();
        let used: BTreeSet<String> = ["quick_open".to_string()].into_iter().collect();
        let seen = vec!["palette".to_string()];
        assert_eq!(pick(&all, &used, &seen).map(|t| t.id), Some("drop-filing"));
    }

    #[test]
    fn once_all_are_seen_the_oldest_returns_and_used_ones_stay_away() {
        let all: Vec<&Tip> = TIPS.iter().collect();
        let mut seen: Vec<String> = TIPS.iter().map(|t| t.id.to_string()).collect();
        let used: BTreeSet<String> = ["quick_open".to_string()].into_iter().collect();
        assert_eq!(pick(&all, &used, &seen).map(|t| t.id), Some("palette"));
        remember(&mut seen, "palette");
        assert_eq!(pick(&all, &used, &seen).map(|t| t.id), Some("drop-filing"));
        assert_eq!(seen.last().map(String::as_str), Some("palette"));
        let few = vec![by_id("quick-open")];
        assert!(pick(&few, &used, &seen).is_none(), "a used feature is never taught");
    }

    #[test]
    fn the_next_tip_differs_from_the_one_just_shown() {
        let all: Vec<&Tip> = TIPS.iter().collect();
        let mut seen = Vec::new();
        let first = pick(&all, &BTreeSet::new(), &seen).map(|t| t.id).expect("a tip");
        remember(&mut seen, first);
        let second = pick(&all, &BTreeSet::new(), &seen).map(|t| t.id).expect("a tip");
        assert_ne!(first, second);
    }
}
