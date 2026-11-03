use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use gpui_kit::RenderImage;

use crate::core::office::{self, OpenError};
use crate::core::pdf_render::{PageRect, PageText, PdfDoc, PdfMeta, fold_query};

/// A page at a resolution, in quarter pixels per point.
pub type PageKey = (usize, u16);

/// The most search hits kept for one query.
const HIT_LIMIT: usize = 5000;
const RENDER_THREADS: usize = 2;

/// One search hit with its marks on the page.
pub struct Hit {
    pub page: usize,
    pub rects: Vec<PageRect>,
    pub excerpt: String,
}

/// Where the pages come from: a PDF, or an Office file laid out as PDF first.
#[derive(Clone)]
pub enum Source {
    Pdf(PathBuf),
    Office(PathBuf),
}

impl Source {
    fn load(&self) -> Result<Vec<u8>, Option<OpenError>> {
        match self {
            Self::Pdf(path) => std::fs::read(path).map_err(|_| None),
            Self::Office(path) => std::panic::catch_unwind(|| office::render_pdf(path)).unwrap_or(Err(OpenError::Broken)).map_err(Some),
        }
    }
}

pub enum Message {
    Opened(Vec<(f32, f32)>),
    Key(String),
    Failed(Option<OpenError>),
    Page(PageKey, Arc<RenderImage>),
    Meta(PdfMeta),
    Text(usize, Arc<PageText>),
    Hits(u64, Vec<Hit>),
    SearchProgress(u64, usize),
    Clipboard(String),
}

pub enum TextJob {
    Page(usize),
    Copy { from: (usize, usize), to: (usize, usize) },
    Search { generation: u64, query: String },
}

#[derive(Default)]
struct Queue {
    wanted: Vec<PageKey>,
    running: HashSet<PageKey>,
    closed: bool,
}

/// The pages the view wants next. Replacing the list drops pages that scrolled away before they started.
pub struct RenderQueue {
    state: Mutex<Queue>,
    ready: Condvar,
}

impl RenderQueue {
    fn new() -> Self {
        Self { state: Mutex::new(Queue::default()), ready: Condvar::new() }
    }

    pub fn replace(&self, wanted: Vec<PageKey>) {
        if let Ok(mut q) = self.state.lock() {
            q.wanted = wanted;
            q.wanted.reverse();
            self.ready.notify_all();
        }
    }

    pub fn close(&self) {
        if let Ok(mut q) = self.state.lock() {
            q.closed = true;
            q.wanted.clear();
            self.ready.notify_all();
        }
    }

    fn next(&self) -> Option<PageKey> {
        let mut q = self.state.lock().ok()?;
        loop {
            if q.closed {
                return None;
            }
            while let Some(key) = q.wanted.pop() {
                if q.running.insert(key) {
                    return Some(key);
                }
            }
            q = self.ready.wait(q).ok()?;
        }
    }

    fn finished(&self, key: PageKey) {
        if let Ok(mut q) = self.state.lock() {
            q.running.remove(&key);
        }
    }
}

fn to_image(p: crate::core::pdf_render::PagePixels) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(p.width, p.height, p.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

fn render_thread(data: Arc<Vec<u8>>, queue: Arc<RenderQueue>, results: async_channel::Sender<Message>) {
    let Some(doc) = PdfDoc::from_data(data) else { return };
    while let Some(key) = queue.next() {
        let image = doc.render_pixels(key.0, f32::from(key.1) / 4.).and_then(to_image);
        queue.finished(key);
        if let Some(image) = image
            && results.send_blocking(Message::Page(key, image)).is_err()
        {
            return;
        }
    }
}

fn search(data: Arc<Vec<u8>>, generation: u64, query: String, latest: Arc<AtomicU64>, results: async_channel::Sender<Message>) {
    let Some(doc) = PdfDoc::from_data(data) else { return };
    let folded = fold_query(&query);
    let mut total = 0;
    for page in 0..doc.page_count() {
        if latest.load(Ordering::Relaxed) != generation {
            return;
        }
        let text = doc.page_text(page);
        let hits: Vec<Hit> = text.find(&folded).into_iter().map(|(start, end)| Hit { page, rects: text.rects(start, end), excerpt: text.excerpt(start, end) }).collect();
        total += hits.len();
        if !hits.is_empty() && results.send_blocking(Message::Hits(generation, hits)).is_err() {
            return;
        }
        if total >= HIT_LIMIT {
            break;
        }
        if (page % 25 == 0 || page + 1 == doc.page_count()) && results.send_blocking(Message::SearchProgress(generation, page + 1)).is_err() {
            return;
        }
    }
    let _ = results.send_blocking(Message::SearchProgress(generation, doc.page_count()));
}

fn copy_range(doc: &PdfDoc, from: (usize, usize), to: (usize, usize)) -> String {
    let mut parts = Vec::new();
    for page in from.0..=to.0 {
        let text = doc.page_text(page);
        let start = if page == from.0 { from.1 } else { 0 };
        let end = if page == to.0 { to.1 } else { text.glyph_count() };
        parts.push(text.slice(start, end));
    }
    parts.join("\n")
}

/// Opens the file off the main thread and serves pages, text, search and metadata from background threads.
pub fn start(source: Source, results: async_channel::Sender<Message>) -> (Arc<RenderQueue>, async_channel::Sender<TextJob>, Arc<AtomicU64>) {
    let queue = Arc::new(RenderQueue::new());
    let (jobs_tx, jobs_rx) = async_channel::unbounded::<TextJob>();
    let latest = Arc::new(AtomicU64::new(0));
    let (q, generation) = (queue.clone(), latest.clone());
    std::thread::spawn(move || {
        let data = match source.load() {
            Ok(data) => Arc::new(data),
            Err(why) => {
                let _ = results.send_blocking(Message::Failed(why));
                return;
            }
        };
        let Some(doc) = PdfDoc::from_data(data.clone()) else {
            let _ = results.send_blocking(Message::Failed(matches!(source, Source::Office(_)).then_some(OpenError::Broken)));
            return;
        };
        let sizes = (0..doc.page_count()).filter_map(|i| doc.page_size(i)).collect();
        if results.send_blocking(Message::Opened(sizes)).is_err() {
            return;
        }
        if results.send_blocking(Message::Key(crate::core::highlights::document_key(&data))).is_err() {
            return;
        }
        for _ in 0..RENDER_THREADS {
            let (data, q, results) = (data.clone(), q.clone(), results.clone());
            std::thread::spawn(move || render_thread(data, q, results));
        }
        if results.send_blocking(Message::Meta(doc.meta(&data))).is_err() {
            return;
        }
        while let Ok(job) = jobs_rx.recv_blocking() {
            let message = match job {
                TextJob::Page(page) => Message::Text(page, Arc::new(doc.page_text(page))),
                TextJob::Copy { from, to } => Message::Clipboard(copy_range(&doc, from, to)),
                TextJob::Search { generation: g, query } => {
                    let (data, latest, results) = (data.clone(), generation.clone(), results.clone());
                    std::thread::spawn(move || search(data, g, query, latest, results));
                    continue;
                }
            };
            if results.send_blocking(message).is_err() {
                return;
            }
        }
    });
    (queue, jobs_tx, latest)
}
