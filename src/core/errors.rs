use std::io::ErrorKind;

use super::i18n::tr;

/// Turns a file error into what happened and what to try, keeping the system's text for anything unexpected.
pub fn explain(error: &anyhow::Error) -> String {
    let io = error.downcast_ref::<std::io::Error>();
    let busy = io.and_then(std::io::Error::raw_os_error).is_some_and(|code| if cfg!(windows) { matches!(code, 32 | 33) } else { matches!(code, 16 | 26) });
    match io.map(std::io::Error::kind) {
        _ if busy => tr("Die Datei wird gerade verwendet. Schließe sie in der anderen App und versuche es erneut."),
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
    fn a_file_open_in_another_program_says_so_in_plain_words() {
        let code = if cfg!(windows) { 32 } else { 16 };
        let busy = anyhow::Error::new(std::io::Error::from_raw_os_error(code));
        let text = explain(&busy);
        assert!(text.contains("verwendet") && !text.contains("os error"), "{text}");
    }

    #[test]
    fn unknown_errors_keep_their_text() {
        assert_eq!(explain(&anyhow::anyhow!("kaputt")), "kaputt");
    }
}
