use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::*;

/// Decoded pixels kept for pictures in notes before the least recently drawn ones go.
const BUDGET: usize = 128 * 1024 * 1024;
/// Pictures drawn this recently stay, so a page full of them never reloads while it shows.
const KEEP: Duration = Duration::from_secs(2);
/// Longest side a note picture is decoded at, at least; wider windows decode up to their own width in pixels.
const LONGEST: u32 = 2048;
const SHRUNK: [&str; 5] = ["png", "jpg", "jpeg", "bmp", "tiff"];

enum Slot {
    Loading { _task: Task<()>, viewers: Vec<EntityId> },
    Done(Result<Arc<RenderImage>, ImageCacheError>),
    Asset(ImageCacheItem),
}

struct Entry {
    slot: Slot,
    used: Instant,
    bytes: usize,
}

/// Pictures in notes: decoded no larger than they are drawn, and let go of when unused once they take too much memory.
pub struct NoteImages {
    items: HashMap<u64, Entry>,
    me: WeakEntity<Self>,
    evicting: bool,
}

struct Shared(Entity<NoteImages>);

impl Global for Shared {}

/// The app-wide cache for pictures in notes, to wrap them in with `image_cache`.
pub fn shared(cx: &mut App) -> Entity<NoteImages> {
    if let Some(s) = cx.try_global::<Shared>() {
        return s.0.clone();
    }
    let entity = cx.new(|cx| NoteImages { items: HashMap::new(), me: cx.weak_entity(), evicting: false });
    cx.set_global(Shared(entity.clone()));
    entity
}

/// Provides the shared cache during layout, where no `App` was at hand when the element was made.
pub struct Notes;

impl ImageCacheProvider for Notes {
    fn provide(&mut self, _: &mut Window, cx: &mut App) -> AnyImageCache {
        shared(cx).into()
    }
}

fn decode(path: &Path, longest: u32) -> Result<Arc<RenderImage>, ImageCacheError> {
    let bytes = std::fs::read(path).map_err(|e| ImageCacheError::Io(Arc::new(e)))?;
    let other = |e: image::ImageError| ImageCacheError::Other(Arc::new(anyhow::anyhow!(e)));
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().map_err(|e| ImageCacheError::Io(Arc::new(e)))?.into_decoder().map_err(other)?;
    let orientation = image::ImageDecoder::orientation(&mut decoder).map_err(other)?;
    let mut picture = image::DynamicImage::from_decoder(decoder).map_err(other)?;
    picture.apply_orientation(orientation);
    if picture.width().max(picture.height()) > longest {
        picture = picture.resize(longest, longest, image::imageops::FilterType::Triangle);
    }
    let mut data = picture.into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(data)])))
}

fn bytes_of(image: &RenderImage) -> usize {
    (0..image.frame_count()).map(|i| image.as_bytes(i).map_or(0, <[u8]>::len)).sum()
}

impl NoteImages {
    /// Lets go of pictures over the budget once the frame is drawn, so pictures this frame showed are counted as drawn.
    fn evict_later(&mut self, cx: &mut App) {
        if std::mem::replace(&mut self.evicting, true) {
            return;
        }
        let me = self.me.clone();
        cx.defer(move |cx| {
            let _ = me.update(cx, |images, cx| {
                images.evicting = false;
                images.evict(cx);
            });
        });
    }

    fn evict(&mut self, cx: &mut App) {
        let mut total: usize = self.items.values().map(|e| e.bytes).sum();
        if total <= BUDGET {
            return;
        }
        let now = Instant::now();
        let mut old: Vec<(u64, Instant)> = self.items.iter().filter(|(_, e)| e.bytes > 0 && now.duration_since(e.used) > KEEP).map(|(k, e)| (*k, e.used)).collect();
        old.sort_by_key(|(_, used)| *used);
        for (key, _) in old {
            if total <= BUDGET {
                break;
            }
            let Some(entry) = self.items.remove(&key) else { continue };
            total -= entry.bytes;
            let image = match entry.slot {
                Slot::Done(Ok(image)) => Some(image),
                Slot::Asset(item) => item.get().and_then(Result::ok),
                _ => None,
            };
            if let Some(image) = image {
                cx.drop_image(image, None);
            }
        }
    }
}

impl ImageCache for NoteImages {
    fn load(&mut self, resource: &Resource, window: &mut Window, cx: &mut App) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        resource.hash(&mut hasher);
        let key = hasher.finish();
        let now = Instant::now();
        if let Some(entry) = self.items.get_mut(&key) {
            entry.used = now;
            return match &mut entry.slot {
                Slot::Loading { viewers, .. } => {
                    let view = window.current_view();
                    if !viewers.contains(&view) {
                        viewers.push(view);
                    }
                    None
                }
                Slot::Done(result) => Some(result.clone()),
                Slot::Asset(item) => {
                    let result = item.use_image(window);
                    if entry.bytes == 0
                        && let Some(Ok(image)) = &result
                    {
                        entry.bytes = bytes_of(image);
                        self.evict_later(cx);
                    }
                    result
                }
            };
        }
        let shrink = match resource {
            Resource::Path(path) => path.extension().map(|e| e.to_string_lossy().to_lowercase()).filter(|e| SHRUNK.contains(&e.as_str())).map(|_| path.clone()),
            _ => None,
        };
        let slot = match shrink {
            Some(path) => {
                let me = self.me.clone();
                let longest = LONGEST.max((f32::from(window.viewport_size().width) * window.scale_factor()).ceil() as u32);
                let task = cx.spawn(async move |cx| {
                    let result = cx.background_executor().spawn(async move { decode(&path, longest) }).await;
                    let viewers = me
                        .update(cx, |images, cx| {
                            let Some(entry) = images.items.get_mut(&key) else { return Vec::new() };
                            entry.bytes = result.as_ref().map_or(0, |i| bytes_of(i));
                            let viewers = match std::mem::replace(&mut entry.slot, Slot::Done(result)) {
                                Slot::Loading { viewers, .. } => viewers,
                                _ => Vec::new(),
                            };
                            images.evict_later(cx);
                            viewers
                        })
                        .unwrap_or_default();
                    cx.update(|cx| {
                        for view in viewers {
                            cx.notify(view);
                        }
                    });
                });
                Slot::Loading { _task: task, viewers: vec![window.current_view()] }
            }
            None => Slot::Asset(ImageCacheItem::new(resource, cx)),
        };
        self.items.insert(key, Entry { slot, used: now, bytes: 0 });
        self.evict_later(cx);
        match &self.items.get(&key)?.slot {
            Slot::Asset(item) => item.use_image(window),
            _ => None,
        }
    }
}
