pub mod bases;
pub mod canvas;
pub mod config;
pub mod frontmatter;
pub mod graph;
pub mod index;
pub mod links;
pub mod moment;
pub mod query;
pub mod render;
pub mod resolve;
pub mod syntax;

#[cfg(test)]
mod reference {
    use super::canvas::Canvas;
    use super::index::{facts_of, rebuild_for_test};
    use crate::core::note_buffer::NoteBuffer;
    use std::path::{Path, PathBuf};

    fn vault() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/vault/testdata")
    }

    #[test]
    fn every_link_of_the_reference_vault_resolves_as_intended() {
        let index = rebuild_for_test(&vault(), |t| facts_of(t, 0.));
        assert_eq!(index.missing.keys().cloned().collect::<Vec<_>>(), ["Fehlt noch"]);
        let target = vault().join("Ziel.md");
        assert!(index.backlinks[&target].iter().any(|m| m.source == vault().join("Syntax.md")));
        assert_eq!(index.notes[&target].aliases, ["Zielnotiz"]);
        assert!(index.tag_tree().iter().any(|(t, n)| t == "projekt" && *n == 2));
        assert!(!index.tag_tree().iter().any(|(t, _)| t == "123"));
    }

    #[test]
    fn opening_and_saving_keeps_every_byte() {
        let dir = std::env::temp_dir().join(format!("schrumbi-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cases: [(&str, &[u8]); 5] = [("crlf", b"# A\r\n\r\nText\r\n"), ("bom", b"\xef\xbb\xbf# A\n"), ("ohne-ende", b"# A\nText"), ("nfd", "Mu\u{308}ller\n".as_bytes()), ("frontmatter", b"---\na: 1\n---\n# A\n")];
        for (name, bytes) in cases {
            let path = dir.join(format!("{name}.md"));
            std::fs::write(&path, bytes).unwrap();
            let mut buffer = NoteBuffer::open(&path);
            let text = buffer.text.clone();
            buffer.restore(text).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name}");
        }
    }

    #[test]
    fn the_canvas_survives_reading_and_writing() {
        let source = std::fs::read_to_string(vault().join("Karte.canvas")).unwrap();
        let doc = Canvas::parse(&source).unwrap();
        assert_eq!((doc.nodes.len(), doc.edges.len()), (4, 1));
        let again = Canvas::parse(&doc.to_json()).unwrap();
        assert_eq!(doc.to_json(), again.to_json());
        let with_unknown = source.replacen("\"id\":\"a1\"", "\"id\":\"a1\",\"plugin\":{\"k\":1}", 1);
        assert!(Canvas::parse(&with_unknown).unwrap().to_json().contains("\"plugin\":{\"k\":1}"));
    }

    #[test]
    fn every_base_of_the_reference_vault_parses() {
        let source = std::fs::read_to_string(vault().join("Tabelle.base")).unwrap();
        let base = super::bases::Base::parse(&source).unwrap();
        assert_eq!(base.views.iter().map(|v| v.kind.as_str()).collect::<Vec<_>>(), ["table", "cards", "list"]);
    }

    #[test]
    fn reading_the_reference_settings_uses_obsidians_keys() {
        let settings = super::config::VaultConfig::read(&vault());
        assert_eq!(settings.attachment_folder_path, "Anhänge");
        assert_eq!(settings.daily.format, "YYYY-MM-DD");
        assert_eq!(settings.templates.folder, "Vorlagen");
        assert_eq!(super::config::bookmarks(&vault()).len(), 3);
    }
}
