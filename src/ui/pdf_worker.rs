use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use gpui_kit::RenderImage;

use crate::core::office::{self, OpenError};
use crate::core::pdf_render::{MarkPaint, PagePixels, PageRect, PageText, PdfDoc, PdfMeta, fold_query, marks_hash, multiply_marks};

/// A page at a resolution, in quarter pixels per point.
pub type PageKey = (usize, u16);

/// The most search hits kept for one query.
const HIT_LIMIT: usize = 5000;
const RENDER_THREADS: usize = 2;
const RENDER_TIMEOUT: Duration = Duration::from_secs(20);
const RENDER_POLL: Duration = Duration::from_millis(100);

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
            Self::Office(path) => crate::core::crash::caught(|| match crate::core::crash::caught_pool() {
                Some(pool) => pool.install(|| office::render_pdf(path)),
                None => office::render_pdf(path),
            })
            .unwrap_or(Err(OpenError::Broken)).map_err(Some),
        }
    }
}

pub enum Message {
    Opened(Vec<(f32, f32)>),
    Key(String),
    Failed(Option<OpenError>),
    Page(PageKey, Arc<RenderImage>, u64, Vec<u64>),
    PageFailed(PageKey, u64),
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
    abandoned: HashSet<usize>,
    closed: bool,
}

/// The pages the view wants next. Replacing the list drops pages that scrolled away before they started.
pub struct RenderQueue {
    state: Mutex<Queue>,
    ready: Condvar,
    marks: Mutex<Arc<HashMap<usize, Vec<MarkPaint>>>>,
}

impl RenderQueue {
    fn new() -> Self {
        Self { state: Mutex::new(Queue::default()), ready: Condvar::new(), marks: Mutex::new(Arc::default()) }
    }

    pub fn replace(&self, wanted: Vec<PageKey>) {
        if let Ok(mut q) = self.state.lock() {
            q.wanted = wanted;
            q.wanted.reverse();
            self.ready.notify_all();
        }
    }

    /// Replaces the highlights that render threads multiply into pages they render from now on.
    pub fn set_marks(&self, marks: Arc<HashMap<usize, Vec<MarkPaint>>>) {
        if let Ok(mut m) = self.marks.lock() {
            *m = marks;
        }
    }

    fn marks(&self) -> Arc<HashMap<usize, Vec<MarkPaint>>> {
        self.marks.lock().map(|m| m.clone()).unwrap_or_default()
    }

    pub fn close(&self) {
        if let Ok(mut q) = self.state.lock() {
            q.closed = true;
            q.wanted.clear();
            self.ready.notify_all();
        }
    }

    /// Marks a page whose render ran past the timeout, so no second render of it starts while the first still holds memory.
    fn abandon(&self, page: usize) {
        if let Ok(mut q) = self.state.lock() {
            q.abandoned.insert(page);
        }
    }

    fn is_abandoned(&self, page: usize) -> bool {
        self.state.lock().map(|q| q.abandoned.contains(&page)).unwrap_or(true)
    }

    fn release(&self, page: usize) {
        if let Ok(mut q) = self.state.lock() {
            q.abandoned.remove(&page);
        }
    }

    fn is_closed(&self) -> bool {
        self.state.lock().map(|q| q.closed).unwrap_or(true)
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

    /// Frees a finished page. When the highlights changed since it was baked, the page is queued again at once,
    /// so a request that arrived while it ran is not lost.
    fn finished(&self, key: PageKey, baked: u64) {
        if let Ok(mut q) = self.state.lock() {
            q.running.remove(&key);
            if self.page_hash(key.0) != baked {
                q.wanted.retain(|k| *k != key);
                q.wanted.push(key);
                self.ready.notify_all();
            }
        }
    }

    fn page_hash(&self, page: usize) -> u64 {
        marks_hash(self.marks().get(&page).map(Vec::as_slice).unwrap_or_default())
    }
}

fn to_image(p: PagePixels) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(p.width, p.height, p.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// Renders on a helper thread so a panic or a page that never finishes only fails that page; `None` also when the queue closes meanwhile.
fn render_guarded(doc: &Arc<PdfDoc>, queue: &Arc<RenderQueue>, page: usize, scale: f32, timeout: Duration) -> Option<PagePixels> {
    if queue.is_abandoned(page) {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let (doc, queue_of_helper) = (doc.clone(), queue.clone());
    std::thread::Builder::new()
        .spawn(move || {
            let rendered = crate::core::crash::caught(|| doc.render_pixels(page, scale)).flatten();
            let _ = tx.send(rendered);
            queue_of_helper.release(page);
        })
        .ok()?;
    let mut waited = Duration::ZERO;
    while waited < timeout && !queue.is_closed() {
        match rx.recv_timeout(RENDER_POLL) {
            Ok(rendered) => return rendered,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => waited += RENDER_POLL,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
    queue.abandon(page);
    if rx.try_recv().is_ok() {
        queue.release(page);
    }
    None
}

fn render_thread(data: Arc<Vec<u8>>, queue: Arc<RenderQueue>, results: async_channel::Sender<Message>) {
    let Some(doc) = PdfDoc::from_data(data).map(Arc::new) else { return };
    while let Some(key) = queue.next() {
        let requested = f32::from(key.1) / 4.;
        let scale = doc.fit_scale(key.0, requested).unwrap_or(requested);
        let rendered = render_guarded(&doc, &queue, key.0, scale, RENDER_TIMEOUT);
        let marks = queue.marks();
        let page_marks = marks.get(&key.0).map(Vec::as_slice).unwrap_or_default();
        let baked = marks_hash(page_marks);
        let image = rendered.and_then(|mut pixels| {
            multiply_marks(&mut pixels, scale, page_marks);
            to_image(pixels)
        });
        let message = match image {
            Some(image) => Message::Page(key, image, baked, page_marks.iter().map(|m| m.sig).collect()),
            None => Message::PageFailed(key, baked),
        };
        let sent = results.send_blocking(message).is_ok();
        queue.finished(key, baked);
        if !sent {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(page: usize, color: [u8; 3]) -> Arc<HashMap<usize, Vec<MarkPaint>>> {
        Arc::new(HashMap::from([(page, vec![MarkPaint::new(1, &[[0., 0., 5., 5.]], color)])]))
    }

    fn baked(queue: &RenderQueue, page: usize) -> u64 {
        queue.page_hash(page)
    }

    #[test]
    fn finished_with_unchanged_marks_only_frees_the_page() {
        let queue = RenderQueue::new();
        queue.set_marks(layer(0, [1, 2, 3]));
        queue.replace(vec![(0, 8)]);
        assert_eq!(queue.next(), Some((0, 8)));
        let hash = baked(&queue, 0);
        queue.finished((0, 8), hash);
        let q = queue.state.lock().unwrap();
        assert!(q.running.is_empty());
        assert!(q.wanted.is_empty());
    }

    #[test]
    fn finished_after_marks_changed_queues_the_page_again_first() {
        let queue = RenderQueue::new();
        queue.set_marks(layer(0, [1, 2, 3]));
        queue.replace(vec![(0, 8), (1, 8)]);
        assert_eq!(queue.next(), Some((0, 8)));
        let hash = baked(&queue, 0);
        queue.set_marks(layer(0, [4, 5, 6]));
        queue.replace(vec![(0, 8), (1, 8)]);
        queue.finished((0, 8), hash);
        assert_eq!(queue.next(), Some((0, 8)));
        assert_eq!(queue.next(), Some((1, 8)));
    }

    #[test]
    fn a_request_made_while_running_is_not_lost() {
        let queue = RenderQueue::new();
        queue.replace(vec![(0, 8)]);
        assert_eq!(queue.next(), Some((0, 8)));
        queue.set_marks(layer(0, [9, 9, 9]));
        queue.replace(vec![(0, 8)]);
        let discarded = queue.state.lock().unwrap().wanted.pop();
        assert_eq!(discarded, Some((0, 8)));
        queue.finished((0, 8), 0);
        let q = queue.state.lock().unwrap();
        assert_eq!(q.wanted, vec![(0, 8)]);
        assert!(q.running.is_empty());
    }

    #[test]
    fn marks_on_other_pages_do_not_requeue() {
        let queue = RenderQueue::new();
        queue.replace(vec![(0, 8)]);
        assert_eq!(queue.next(), Some((0, 8)));
        queue.set_marks(layer(3, [9, 9, 9]));
        queue.finished((0, 8), 0);
        assert!(queue.state.lock().unwrap().wanted.is_empty());
    }

    fn sized_page_data(content: &str) -> Arc<Vec<u8>> {
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let contents = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => contents });
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        Arc::new(out)
    }

    #[test]
    fn a_failing_page_does_not_stop_the_queue() {
        let data = sized_page_data("[0.0000001] 0 d 1000 w 0 0 m 600 700 l S");
        let queue = Arc::new(RenderQueue::new());
        let (tx, rx) = async_channel::unbounded();
        let q = queue.clone();
        let worker = std::thread::spawn(move || render_thread(data, q, tx));
        queue.replace(vec![(0, 4), (0, 8), (5, 4)]);
        for _ in 0..3 {
            assert!(matches!(rx.recv_blocking().unwrap(), Message::PageFailed(..)));
        }
        queue.close();
        worker.join().unwrap();
    }

    #[test]
    fn a_page_that_never_finishes_times_out_and_closing_stops_waiting() {
        let doc = Arc::new(PdfDoc::from_data(sized_page_data("0 0 m 10 10 l S")).unwrap());
        let queue = Arc::new(RenderQueue::new());
        assert!(render_guarded(&doc, &queue, 0, 1., Duration::from_secs(10)).is_some());
        assert!(render_guarded(&doc, &queue, 0, 1., Duration::ZERO).is_none());
        while queue.is_abandoned(0) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(render_guarded(&doc, &queue, 0, 1., Duration::from_secs(10)).is_some());
        queue.close();
        assert!(render_guarded(&doc, &queue, 0, 1., Duration::from_secs(10)).is_none());
    }

    #[test]
    fn a_page_still_rendering_after_the_timeout_is_not_started_again() {
        let doc = Arc::new(PdfDoc::from_data(sized_page_data("0 0 m 10 10 l S")).unwrap());
        let queue = Arc::new(RenderQueue::new());
        queue.abandon(0);
        assert!(render_guarded(&doc, &queue, 0, 1., Duration::from_secs(10)).is_none());
        queue.release(0);
        assert!(render_guarded(&doc, &queue, 0, 1., Duration::from_secs(10)).is_some());
    }
}
