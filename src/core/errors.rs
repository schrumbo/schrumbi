use std::io::ErrorKind;

use super::i18n::tr;

/// Turns a file error into what happened and what to try, keeping the system's text for anything unexpected.
pub fn explain(error: &anyhow::Error) -> String {
    match error.downcast_ref::<std::io::Error>().map(std::io::Error::kind) {
        Some(ErrorKind::PermissionDenied) => tr("Keine Berechtigung. Prüfe die Zugriffsrechte des Ordners oder ob die Datei schreibgeschützt ist."),
        Some(ErrorKind::NotFound) => tr("Die Datei wurde nicht gefunden. Sie wurde vielleicht verschoben oder gelöscht."),
        Some(ErrorKind::StorageFull) => tr("Der Speicher ist voll. Gib Speicherplatz frei und versuche es erneut."),
        Some(ErrorKind::AlreadyExists) => tr("Eine Datei mit diesem Namen existiert bereits. Wähle einen anderen Namen."),
        Some(ErrorKind::ResourceBusy) => tr("Die Datei wird gerade verwendet. Schließe sie in der anderen App und versuche es erneut."),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_file_errors_say_what_to_do() {
        let denied = anyhow::Error::new(std::io::Error::from(ErrorKind::PermissionDenied));
        assert!(explain(&denied).contains("Berechtigung"));
    }

    #[test]
    fn unknown_errors_keep_their_text() {
        assert_eq!(explain(&anyhow::anyhow!("kaputt")), "kaputt");
    }
}
