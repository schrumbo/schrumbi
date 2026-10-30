use std::path::Path;
use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

/// A loaded PDF whose pages render on demand.
pub struct PdfDoc {
    pdf: Pdf,
}

impl PdfDoc {
    pub fn open(path: &Path) -> Option<Self> {
        let data = std::fs::read(path).ok()?;
        Some(Self { pdf: Pdf::new(Arc::new(data)).ok()? })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Page size in points.
    pub fn page_size(&self, index: usize) -> Option<(f32, f32)> {
        self.pdf.pages().get(index).map(|p| p.render_dimensions())
    }

    /// PNG of one page at `scale` pixels per point, on white.
    pub fn render_png(&self, index: usize, scale: f32) -> Option<Vec<u8>> {
        let page = self.pdf.pages().get(index)?;
        let cache = RenderCache::new();
        let pixmap = hayro::render(page, &cache, &InterpreterSettings::default(), &RenderSettings::default(), &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
        pixmap.into_png().ok()
    }
}
