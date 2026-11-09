//! The media views (Popups, Audio, All media) and the Tags and Artists views. Ported from
//! `pack-editor/src/lib/MediaGrid.svelte`, `AudioList.svelte`, `MediaToolbar.svelte`,
//! `Sidebar.svelte` (the inspector), `MediaViewer.svelte` and `VocabularyPage.svelte`.
//!
//! | Module | What |
//! | --- | --- |
//! | `browser` | One media tab: toolbar, the grid (Popups, All media) or list (Audio), dialogs |
//! | `thumbs` | Thumbnails for the grid: decoded off the main thread, bounded, released off-screen |
//! | `inspector` | The selected file(s): preview, name, source, tags, artists, details, remove |
//! | `viewer` | A full-size view of one file over the window, stepped with the arrow keys |
//! | `tags` | The Tags and Artists views: search, rename, merge, delete, jump to their media |
//!
//! The model they run on (filters, selection, label summaries) is UI-free and tested, in
//! `pack_editor_core::session::library`.

mod browser;
mod inspector;
pub mod tags;
mod thumbs;
mod viewer;

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Entity, EntityId, Global, Window};
use media_gpui::{MediaKind, MediaSource, PreviewSource};
use pack_editor_core::pack::MediaFile;
use pack_editor_core::session::library::{LabelKind, MediaFilter, MediaTab, Selection};
use shared::encode::FileInfo;

use crate::editor::Editor;
use crate::shell::View;

pub fn create(view: View, editor: Entity<Editor>, window: &mut Window, cx: &mut App) -> AnyView {
    match view {
        View::Popups => browser::create(MediaTab::Popups, editor, window, cx),
        View::Audio => browser::create(MediaTab::Audio, editor, window, cx),
        View::AllMedia => browser::create(MediaTab::AllMedia, editor, window, cx),
        View::Artists => cx
            .new(|cx| tags::VocabularyView::new(LabelKind::Artists, editor, window, cx))
            .into(),
        _ => cx
            .new(|cx| tags::VocabularyView::new(LabelKind::Tags, editor, window, cx))
            .into(),
    }
}

pub(crate) fn tab_view(tab: MediaTab) -> View {
    match tab {
        MediaTab::Popups => View::Popups,
        MediaTab::Audio => View::Audio,
        MediaTab::AllMedia => View::AllMedia,
    }
}

// ── What a media tab remembers ──────────────────────────────────────────────────────────────

/// One media tab's filters and selection. The shell drops a media view when you leave it (to
/// release its preview), so this lives outside the view and is restored when it's made again.
#[derive(Clone, Default)]
pub(crate) struct TabState {
    pub filter: MediaFilter,
    pub selection: Selection,
    /// A file to scroll to and select when the tab is next shown.
    pub reveal: Option<u64>,
}

/// Every media tab's [`TabState`] for the open editor (a newly opened pack starts fresh).
#[derive(Default)]
struct TabStates(HashMap<(EntityId, MediaTab), TabState>);

impl Global for TabStates {}

pub(crate) fn load_tab(editor: EntityId, tab: MediaTab, cx: &mut App) -> TabState {
    cx.default_global::<TabStates>()
        .0
        .get(&(editor, tab))
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn store_tab(editor: EntityId, tab: MediaTab, state: TabState, cx: &mut App) {
    let states = &mut cx.default_global::<TabStates>().0;
    // Only one editor is open at a time: whatever another one left behind is for a closed pack.
    states.retain(|(id, _), _| *id == editor);
    states.insert((editor, tab), state);
}

/// Shows `tab` with its filters replaced by `filter` and nothing selected: the Tags and Artists
/// views' "Media" menu.
pub(crate) fn show_filtered(
    editor: EntityId,
    tab: MediaTab,
    filter: MediaFilter,
    window: &mut Window,
    cx: &mut App,
) {
    let mut state = load_tab(editor, tab, cx);
    state.filter = filter;
    state.selection = Selection::default();
    store_tab(editor, tab, state, cx);
    crate::shell::show_view(tab_view(tab), window, cx);
}

/// Shows `tab` with `id` selected and scrolled to, clearing the tab's filters if they hide it:
/// the inspector's "Used as" links.
pub(crate) fn reveal(
    editor: &Entity<Editor>,
    tab: MediaTab,
    id: u64,
    window: &mut Window,
    cx: &mut App,
) {
    let editor_id = editor.entity_id();
    let mut state = load_tab(editor_id, tab, cx);
    let session = editor.read(cx).session();
    if !state.filter.apply(tab, session.media().iter()).contains(&id) {
        state.filter.clear();
    }
    state.selection.select_single(id);
    state.reveal = Some(id);
    store_tab(editor_id, tab, state, cx);
    crate::shell::show_view(tab_view(tab), window, cx);
}

// ── Opening a file's bytes for a preview ────────────────────────────────────────────────────

/// Resolves `file`'s bytes as a bounded range of the pack (or its loose copy) for `MediaView`.
/// The returned future runs the IO on the tokio runtime; await it from a gpui task. No server and
/// no temporary copy; see `media-gpui/README.md`.
pub(crate) fn open_preview(
    editor: &Editor,
    file: &MediaFile,
) -> impl Future<Output = anyhow::Result<PreviewSource>> + 'static {
    let id = file.id;
    let (kind, duration) = match file.file_info {
        FileInfo::Image { .. } => (MediaKind::Image, None),
        FileInfo::Video {
            transparent,
            duration,
            ..
        } => (MediaKind::Video { transparent }, Some(duration)),
        FileInfo::Audio { duration } => (MediaKind::Audio, Some(duration)),
    };
    let view = editor.pack().get_view();
    async move {
        let view = view?;
        crate::runtime::run(async move {
            let (range, _) = view
                .open_file_range(
                    id,
                    pack_editor_core::pack::Range {
                        start: Some(0),
                        end: None,
                    },
                )
                .await?;
            let (file, offset, length) = range.into_file_range().await?;
            Ok::<_, anyhow::Error>(PreviewSource {
                media: MediaSource::from_file(file, offset, length),
                kind,
                duration: duration
                    .filter(|d| d.is_finite() && *d >= 0.)
                    .map(Duration::from_secs_f64),
            })
        })
        .await
        .and_then(|result| result)
    }
}

// ── Readouts ────────────────────────────────────────────────────────────────────────────────

/// `m:ss`, or `h:mm:ss` past an hour.
pub(crate) fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0. {
        return "0:00".into();
    }
    let total = seconds.round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub(crate) fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if value >= 100. {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(crate) fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}
