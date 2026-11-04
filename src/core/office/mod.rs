//! Read-only views of Office files: Word and PowerPoint become PDF pages through Typst, spreadsheets a grid.

pub mod docx;
pub mod numfmt;
mod package;
pub mod pptx;
pub mod sheet;
mod typ;
mod xml;

use std::path::Path;

use super::i18n::tr;

/// How a file is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Document,
    Slides,
    Sheet,
}

/// Why a file could not be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    Broken,
    Locked,
}

impl OpenError {
    /// Title and explanation for the error state.
    pub fn describe(self) -> (String, String) {
        match self {
            Self::Locked => (tr("Mit Passwort geschützt"), tr("Schrumbi kann verschlüsselte Dateien nicht anzeigen. Öffne sie in der Standard-App.")),
            Self::Broken => (tr("Datei konnte nicht geöffnet werden"), tr("Die Datei ist beschädigt oder hat ein unbekanntes Format.")),
        }
    }
}

/// The view an Office file gets, if Schrumbi can show it.
pub fn kind(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "docx" | "docm" | "dotx" => Some(Kind::Document),
        "pptx" | "pptm" | "ppsx" | "potx" => Some(Kind::Slides),
        "xlsx" | "xlsm" | "xltx" | "xls" | "xlsb" | "ods" => Some(Kind::Sheet),
        _ => None,
    }
}

/// Renders a Word or PowerPoint file to PDF bytes; a broken picture costs only the picture.
pub fn render_pdf(path: &Path) -> Result<Vec<u8>, OpenError> {
    let bytes = std::fs::read(path).map_err(|_| OpenError::Broken)?;
    let source = match kind(path) {
        Some(Kind::Document) => docx::to_typst(&docx::parse(bytes)?),
        Some(Kind::Slides) => pptx::to_typst(&pptx::parse(bytes)?),
        _ => return Err(OpenError::Broken),
    };
    super::pdf_export::compile(&source.text, source.files.clone()).or_else(|_| super::pdf_export::compile(&source.without_images(), Vec::new())).map_err(|_| OpenError::Broken)
}

/// Typst source with the files it reads.
pub struct Source {
    pub text: String,
    pub files: Vec<(String, Vec<u8>)>,
}

impl Source {
    fn without_images(&self) -> String {
        let mut text = self.text.clone();
        for (name, _) in &self.files {
            text = text.replace(&format!("image(\"{name}\""), "office-missing(\"\"");
        }
        text
    }
}

