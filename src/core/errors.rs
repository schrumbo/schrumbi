use std::io::ErrorKind;

use super::i18n::tr;

/// Turns a file error into what happened and what to try, keeping the system's text for anything unexpected.
pub fn explain(error: &anyhow::Error) -> String {
    error.downcast_ref::<std::io::Error>().and_then(explain_io).unwrap_or_else(|| error.to_string())
}

/// What `explain` says for a file error it knows, in plain German with the next step; `None` for anything unexpected.
pub fn explain_io(io: &std::io::Error) -> Option<String> {
    let busy = io.raw_os_error().is_some_and(|code| if cfg!(windows) { matches!(code, 32 | 33) } else { matches!(code, 16 | 26) });
    match io.kind() {
        _ if busy => Some(tr("Die Datei wird gerade verwendet. Schließe sie in der anderen App und versuche es erneut.")),
        ErrorKind::PermissionDenied => Some(tr("Keine Berechtigung. Prüfe die Zugriffsrechte des Ordners oder ob die Datei schreibgeschützt ist.")),
        ErrorKind::NotFound => Some(tr("Die Datei wurde nicht gefunden. Sie wurde vielleicht verschoben oder gelöscht.")),
        ErrorKind::StorageFull => Some(tr("Der Speicher ist voll. Gib Speicherplatz frei und versuche es erneut.")),
        ErrorKind::AlreadyExists => Some(tr("Eine Datei mit diesem Namen existiert bereits. Wähle einen anderen Namen.")),
        ErrorKind::ResourceBusy => Some(tr("Die Datei wird gerade verwendet. Schließe sie in der anderen App und versuche es erneut.")),
        _ => None,
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
