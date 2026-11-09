//! Thumbnails for the media grid: the small WebP import stores per file, decoded off the main
//! thread into gpui images, at most [`CAPACITY`] at a time.
//!
//! The grid asks for the files it's drawing ([`Thumbnails::want`]) every frame. Missing ones are
//! fetched and decoded in batches on the background executor, one batch at a time and only for
//! files still wanted when the batch starts, so scrolling past ten thousand files never queues
//! ten thousand decodes. Once the cache is over capacity, the least recently drawn images not on
//! screen are dropped from the atlas as well as the cache.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::{App, AppContext, RenderImage, WeakEntity, Window};
use pack_editor_core::pack::{ops, MediaPack};

/// About four screens of a full-HD grid; 100x75 thumbnails are ~30 KB each, so ~15 MB.
const CAPACITY: usize = 480;
/// Files per background fetch: one SQLite query and as many small decodes.
const BATCH: usize = 48;

enum Slot {
    Ready { image: Arc<RenderImage>, used: u64 },
    /// No thumbnail stored, or it didn't decode: drawn as a placeholder, not asked for again.
    Missing,
}

#[derive(Default)]
pub(crate) struct Thumbnails {
    slots: HashMap<u64, Slot>,
    /// Asked for this frame, in drawing order, and not cached yet.
    wanted: Vec<u64>,
    /// Drawn this frame.
    visible: HashSet<u64>,
    loading: bool,
    frame: u64,
    /// Decoded so far, for the headless check's readout.
    pub decoded: usize,
}

impl Thumbnails {
    /// Starts a frame: what was visible is forgotten until the grid asks again.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.visible.clear();
        self.wanted.clear();
    }

    /// The thumbnail for `id` if it's ready, noting that the grid drew (or wants) it.
    pub fn want(&mut self, id: u64) -> Option<Arc<RenderImage>> {
        self.visible.insert(id);
        match self.slots.get_mut(&id) {
            Some(Slot::Ready { image, used }) => {
                *used = self.frame;
                Some(image.clone())
            }
            Some(Slot::Missing) => None,
            None => {
                self.wanted.push(id);
                None
            }
        }
    }

    pub fn is_missing(&self, id: u64) -> bool {
        matches!(self.slots.get(&id), Some(Slot::Missing))
    }

    pub fn cached(&self) -> usize {
        self.slots
            .values()
            .filter(|slot| matches!(slot, Slot::Ready { .. }))
            .count()
    }

    /// Ends a frame: drops what's over capacity, then starts fetching the next batch if none is
    /// running. `owner` is notified when a batch lands, which draws it and asks for more.
    pub fn end_frame<T: 'static>(
        this: &std::rc::Rc<std::cell::RefCell<Self>>,
        pack: &Arc<MediaPack>,
        owner: WeakEntity<T>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut thumbs = this.borrow_mut();
        thumbs.evict(window);
        if thumbs.loading || thumbs.wanted.is_empty() {
            return;
        }
        let batch: Vec<u64> = thumbs.wanted.iter().copied().take(BATCH).collect();
        thumbs.loading = true;
        drop(thumbs);

        let pack = pack.clone();
        let decode = cx.background_spawn(async move {
            let stored = pack
                .connection()
                .and_then(|connection| ops::library::thumbnails(&connection, &batch));
            let mut decoded: HashMap<u64, Arc<RenderImage>> = HashMap::new();
            match stored {
                Ok(stored) => {
                    for (id, bytes) in stored {
                        if let Some(image) = decode(&bytes) {
                            decoded.insert(id, image);
                        }
                    }
                }
                Err(error) => tracing::warn!("couldn't read thumbnails: {error:#}"),
            }
            (batch, decoded)
        });
        let this = this.clone();
        cx.spawn(async move |cx| {
            let (batch, mut decoded) = decode.await;
            {
                let mut thumbs = this.borrow_mut();
                thumbs.loading = false;
                let frame = thumbs.frame;
                for id in batch {
                    let slot = match decoded.remove(&id) {
                        Some(image) => {
                            thumbs.decoded += 1;
                            Slot::Ready { image, used: frame }
                        }
                        None => Slot::Missing,
                    };
                    thumbs.slots.insert(id, slot);
                }
            }
            let _ = owner.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// Drops the least recently drawn images that aren't on screen, down to capacity.
    fn evict(&mut self, window: &mut Window) {
        let ready = self.cached();
        if ready <= CAPACITY {
            return;
        }
        let mut candidates: Vec<(u64, u64)> = self
            .slots
            .iter()
            .filter_map(|(id, slot)| match slot {
                Slot::Ready { used, .. } if !self.visible.contains(id) => Some((*used, *id)),
                _ => None,
            })
            .collect();
        candidates.sort_unstable();
        for (_, id) in candidates.into_iter().take(ready - CAPACITY) {
            if let Some(Slot::Ready { image, .. }) = self.slots.remove(&id) {
                let _ = window.drop_image(image);
            }
        }
    }

    /// Drops every image from the atlas: the grid is going away.
    pub fn clear(&mut self, window: &mut Window) {
        for (_, slot) in self.slots.drain() {
            if let Slot::Ready { image, .. } = slot {
                let _ = window.drop_image(image);
            }
        }
        self.wanted.clear();
        self.visible.clear();
    }
}

/// Decodes a stored thumbnail into the BGRA gpui paints.
fn decode(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let rgba = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut pixels = rgba.into_raw();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let frame = image::Frame::new(image::RgbaImage::from_raw(width, height, pixels)?);
    Some(Arc::new(RenderImage::new(smallvec::smallvec![frame])))
}
