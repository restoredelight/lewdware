//! `Session` methods for media, tags, artists and per-file attributes. One file per track of the
//! gpui port, so tracks don't edit the same file; see `design/editor-model.md`.
//!
//! Each method is one line over [`Session::write`](super::Session) and a function in
//! `crate::pack::ops::library`.
//!
//! Also the media library's model, free of any UI toolkit so it can be tested here: which files a
//! media tab lists and in what order ([`MediaFilter`]), what is selected and how the gestures change
//! it ([`Selection`]), where a key moves in a grid ([`grid_step`]), and how a selection's tags read
//! taken together ([`LabelSummary`]). Ported from `pack-editor/src/lib/store.svelte.ts`,
//! `mediaSelection.svelte.ts` and `labelSummary.ts`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::pack::{ArtistSummary, MediaUsage, TagRow};
use shared::encode::FileInfo;

impl Session {
    // ── Files ───────────────────────────────────────────────────────────────────────────────

    /// Removes files from the pack (soft-deleted, one undo entry). Any slot pointing at one is
    /// emptied with it.
    pub fn remove_files(&mut self, ids: &[u64]) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::library::remove_files(c, ids))
    }

    pub fn rename_file(&mut self, id: u64, name: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::library::set_file_name(c, id, name))
    }

    /// Popup audio when `popup`, background audio otherwise.
    pub fn set_popup_audio(&mut self, ids: &[u64], popup: bool) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::library::set_popup_audio(c, ids, popup))
    }

    /// A track's own level. `None` (or 1.0, which means the same) is "no opinion".
    pub fn set_audio_volume(
        &mut self,
        ids: &[u64],
        volume: Option<f64>,
        label: &str,
    ) -> Result<Applied<()>> {
        let volume = volume.filter(|volume| (*volume - 1.0).abs() > f64::EPSILON);
        let ids = ids.to_vec();
        self.edit(BehaviourAction::new(label, move |tx| {
            shared::behaviour::editor::set_audio_volume(tx, &ids, volume)
        }))
        .map(|applied| Applied {
            value: (),
            changes: applied.changes,
            history: applied.history,
        })
    }

    // ── Reads ───────────────────────────────────────────────────────────────────────────────

    pub fn tag_rows(&self) -> Result<Vec<TagRow>> {
        self.read(ops::library::tag_rows)
    }

    pub fn artist_rows(&self) -> Result<Vec<ArtistSummary>> {
        self.read(ops::library::artist_rows)
    }

    pub fn media_usage(&self, id: u64) -> Result<Vec<MediaUsage>> {
        self.read(|c| ops::library::media_usage(c, id))
    }

    pub fn audio_volume(&self, id: u64) -> Result<Option<f64>> {
        self.read(|c| ops::library::audio_volume(c, id))
    }
}

// ── Which files a media tab lists ───────────────────────────────────────────────────────────

/// The three views that list media. Each has its own filters and selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaTab {
    /// Images and videos the built-in modes spawn as popups.
    Popups,
    Audio,
    /// Everything, including wallpapers, splashes and subliminals.
    AllMedia,
}

impl MediaTab {
    /// Whether the tab lists `file` at all, before its own filters.
    pub fn includes(self, file: &MediaFile) -> bool {
        match self {
            MediaTab::Popups => file.popup_eligible,
            MediaTab::Audio => {
                matches!(file.file_info, FileInfo::Audio { .. }) && !file.explicit_only
            }
            MediaTab::AllMedia => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KindFilter {
    #[default]
    All,
    Image,
    Video,
    Audio,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoleFilter {
    #[default]
    All,
    Background,
    Popup,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    /// Import order.
    #[default]
    Created,
    Name,
    Size,
}

/// One tab's search, filters and sort. `Default` is "everything, oldest first".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaFilter {
    pub query: String,
    pub kind: KindFilter,
    /// Only the Audio tab offers this; it narrows audio files only.
    pub role: RoleFilter,
    /// A file passes if it carries any of these (empty: no filter).
    pub tags: BTreeSet<String>,
    pub artists: BTreeSet<String>,
    pub sort: SortKey,
    pub descending: bool,
}

impl MediaFilter {
    /// Whether anything narrows the list. Sorting doesn't.
    pub fn is_active(&self) -> bool {
        !self.query.is_empty()
            || self.kind != KindFilter::All
            || self.role != RoleFilter::All
            || !self.tags.is_empty()
            || !self.artists.is_empty()
    }

    /// Back to everything, keeping the sort.
    pub fn clear(&mut self) {
        *self = MediaFilter {
            sort: self.sort,
            descending: self.descending,
            ..MediaFilter::default()
        };
    }

    fn matches(&self, file: &MediaFile, query: &str) -> bool {
        let kind_ok = matches!(
            (self.kind, &file.file_info),
            (KindFilter::All, _)
                | (KindFilter::Image, FileInfo::Image { .. })
                | (KindFilter::Video, FileInfo::Video { .. })
                | (KindFilter::Audio, FileInfo::Audio { .. })
        );
        if !kind_ok {
            return false;
        }
        if self.role != RoleFilter::All && matches!(file.file_info, FileInfo::Audio { .. }) {
            let popup = is_popup_audio(file);
            if popup != (self.role == RoleFilter::Popup) {
                return false;
            }
        }
        if !query.is_empty() && !file.file_name.to_lowercase().contains(query) {
            return false;
        }
        if !self.tags.is_empty() && !file.tags.iter().any(|tag| self.tags.contains(tag)) {
            return false;
        }
        if !self.artists.is_empty()
            && !file
                .artists
                .iter()
                .any(|artist| self.artists.contains(artist))
        {
            return false;
        }
        true
    }

    /// The ids `tab` lists under this filter, in its sort order.
    pub fn apply<'a>(
        &self,
        tab: MediaTab,
        files: impl IntoIterator<Item = &'a MediaFile>,
    ) -> Vec<u64> {
        let query = self.query.trim().to_lowercase();
        let mut listed: Vec<&MediaFile> = files
            .into_iter()
            .filter(|file| tab.includes(file) && self.matches(file, &query))
            .collect();
        listed.sort_by(|a, b| {
            let ordering = match self.sort {
                SortKey::Created => a.id.cmp(&b.id),
                SortKey::Name => natural_cmp(&a.file_name, &b.file_name).then(a.id.cmp(&b.id)),
                SortKey::Size => a.size.cmp(&b.size).then(a.id.cmp(&b.id)),
            };
            if self.descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
        listed.into_iter().map(|file| file.id).collect()
    }
}

/// Whether an audio file plays when a popup appears (rather than in the background).
pub fn is_popup_audio(file: &MediaFile) -> bool {
    file.audio_role.as_deref() == Some("popup")
        || file
            .tags
            .iter()
            .any(|tag| tag == shared::tags::POPUP_AUDIO_TAG)
}

/// Case-insensitive, with runs of digits compared as numbers: "file 9" before "file 10". What the
/// webview's `Intl.Collator({ numeric: true, sensitivity: 'base' })` did.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        chars.next();
                    }
                    digits
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let (xt, yt) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let ordering = xt.len().cmp(&yt.len()).then_with(|| xt.cmp(yt));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (Some(x), Some(y)) => {
                let ordering = x.to_lowercase().cmp(y.to_lowercase());
                if ordering != Ordering::Equal {
                    return ordering;
                }
                a.next();
                b.next();
            }
        }
    }
}

// ── Selection ───────────────────────────────────────────────────────────────────────────────

/// What's selected in one media tab, and the state of the gestures building it.
///
/// `primary` is the file the inspector speaks for; `active` is where the keyboard is (the focus
/// ring); `anchor` is the fixed end of a Shift range. Every operation that needs the order of
/// files takes the tab's visible list, so a range or a select-all covers exactly what the author
/// can see.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub selected: BTreeSet<u64>,
    pub primary: Option<u64>,
    pub active: Option<u64>,
    pub anchor: Option<u64>,
}

impl Selection {
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn contains(&self, id: u64) -> bool {
        self.selected.contains(&id)
    }

    /// The selection in `list` order.
    pub fn ordered(&self, list: &[u64]) -> Vec<u64> {
        list.iter()
            .copied()
            .filter(|id| self.selected.contains(id))
            .collect()
    }

    pub fn select_single(&mut self, id: u64) {
        self.selected = BTreeSet::from([id]);
        self.primary = Some(id);
        self.active = Some(id);
        self.anchor = Some(id);
    }

    /// A click: plain replaces, `toggle` (Ctrl/Cmd) adds or removes one, `extend` (Shift) selects
    /// the range from the anchor.
    pub fn click(&mut self, list: &[u64], id: u64, extend: bool, toggle: bool) {
        if extend && self.anchor.is_some() {
            self.select_range(list, self.anchor.unwrap(), id);
        } else if toggle {
            self.toggle(id);
            self.anchor = Some(id);
        } else {
            self.select_single(id);
        }
    }

    pub fn toggle(&mut self, id: u64) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
        self.active = Some(id);
        self.primary = if self.selected.contains(&id) {
            Some(id)
        } else {
            self.selected.iter().next_back().copied()
        };
    }

    /// Everything from `anchor` to `target` in `list`, inclusive. Nothing changes if either isn't
    /// listed.
    pub fn select_range(&mut self, list: &[u64], anchor: u64, target: u64) {
        let (Some(a), Some(t)) = (
            list.iter().position(|&id| id == anchor),
            list.iter().position(|&id| id == target),
        ) else {
            return;
        };
        let (first, last) = if a <= t { (a, t) } else { (t, a) };
        self.selected = list[first..=last].iter().copied().collect();
        self.primary = Some(target);
        self.active = Some(target);
        self.anchor = Some(anchor);
    }

    pub fn select_all(&mut self, list: &[u64]) {
        self.selected = list.iter().copied().collect();
        self.primary = list.last().copied();
        self.active = self.primary;
    }

    /// Deselects everything and forgets the anchor: Escape, a click on empty space.
    pub fn clear(&mut self) {
        *self = Selection::default();
    }

    /// Moves the keyboard to `list[index]`. `extend` grows a range from the anchor (Shift);
    /// `preserve` moves without touching the selection (Ctrl/Cmd), to reach a file to Space-toggle.
    pub fn move_to(&mut self, list: &[u64], index: usize, extend: bool, preserve: bool) {
        let Some(&next) = list.get(index) else {
            return;
        };
        let current = self.active;
        self.active = Some(next);
        if extend {
            let anchor = *self.anchor.get_or_insert(current.unwrap_or(next));
            self.select_range(list, anchor, next);
        } else if !preserve {
            self.select_single(next);
        }
    }

    /// Drops whatever `list` no longer contains: files removed, or hidden by a filter.
    pub fn retain(&mut self, list: &[u64]) {
        let listed: BTreeSet<u64> = list.iter().copied().collect();
        self.selected.retain(|id| listed.contains(id));
        if self.primary.is_some_and(|id| !self.selected.contains(&id)) {
            self.primary = self.selected.iter().next_back().copied();
        }
        if self.active.is_some_and(|id| !listed.contains(&id)) {
            self.active = self.primary;
        }
        if self.anchor.is_some_and(|id| !listed.contains(&id)) {
            self.anchor = None;
        }
    }
}

/// A navigation key in a grid or list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// Where `step` goes from `from` in a list of `len` laid out `columns` wide (1 for a list).
/// `None` when the move goes nowhere: an arrow already at the edge isn't a move, so it shouldn't
/// collapse a selection built up around it. Home and End always are.
pub fn grid_step(step: Step, from: Option<usize>, len: usize, columns: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    let columns = columns.max(1);
    let Some(from) = from else {
        return Some(match step {
            Step::End => last,
            _ => 0,
        });
    };
    let to = match step {
        Step::Right => (from + 1).min(last),
        Step::Left => from.saturating_sub(1),
        Step::Down => (from + columns).min(last),
        Step::Up => from.saturating_sub(columns),
        Step::Home => return Some(0),
        Step::End => return Some(last),
    };
    (to != from).then_some(to)
}

// ── Tags and artists across a selection ─────────────────────────────────────────────────────

/// A selection's tags (or artists) taken together: the names every selected file carries, which
/// the inspector edits as a set, and the ones only some carry, offered as "add to all" or "remove".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelSummary {
    /// Sorted.
    pub common: Vec<String>,
    /// Sorted by name, with how many of the selection carry each.
    pub mixed: Vec<(String, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Tags,
    Artists,
}

impl LabelSummary {
    /// Managed tags are left out: they're the editor's markers, not the author's.
    pub fn of<'a>(files: impl IntoIterator<Item = &'a MediaFile>, kind: LabelKind) -> Self {
        let mut counts = BTreeMap::<&str, usize>::new();
        let mut total = 0;
        for file in files {
            total += 1;
            let names = match kind {
                LabelKind::Tags => &file.tags,
                LabelKind::Artists => &file.artists,
            };
            for name in names {
                if kind == LabelKind::Tags && shared::tags::is_managed(name) {
                    continue;
                }
                *counts.entry(name.as_str()).or_default() += 1;
            }
        }
        let mut summary = LabelSummary::default();
        for (name, count) in counts {
            if count == total {
                summary.common.push(name.to_string());
            } else {
                summary.mixed.push((name.to_string(), count));
            }
        }
        summary
    }
}

/// How many of `files` an add (or remove) of `name` would actually change. An edit that changes
/// none of them isn't made, so it doesn't leave an empty undo entry.
pub fn label_edit_count<'a>(
    files: impl IntoIterator<Item = &'a MediaFile>,
    kind: LabelKind,
    name: &str,
    add: bool,
) -> usize {
    files
        .into_iter()
        .filter(|file| {
            let names = match kind {
                LabelKind::Tags => &file.tags,
                LabelKind::Artists => &file.artists,
            };
            names.iter().any(|n| n == name) != add
        })
        .count()
}

/// Every author-editable tag and every artist on any file in the index, sorted: the suggestions
/// for the inspector and the toolbar's filter menus.
pub fn label_names(index: &MediaIndex, kind: LabelKind) -> Vec<String> {
    let mut names = BTreeSet::new();
    for file in index.iter() {
        match kind {
            LabelKind::Tags => names.extend(
                file.tags
                    .iter()
                    .filter(|tag| !shared::tags::is_managed(tag))
                    .cloned(),
            ),
            LabelKind::Artists => names.extend(file.artists.iter().cloned()),
        }
    }
    let mut names: Vec<String> = names.into_iter().collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    names
}

/// How many files carry each name, per media tab: the Tags and Artists views' "Media" menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabCounts {
    pub popups: usize,
    pub audio: usize,
    pub all: usize,
}

impl TabCounts {
    pub fn get(&self, tab: MediaTab) -> usize {
        match tab {
            MediaTab::Popups => self.popups,
            MediaTab::Audio => self.audio,
            MediaTab::AllMedia => self.all,
        }
    }
}

pub fn label_counts(index: &MediaIndex, kind: LabelKind) -> BTreeMap<String, TabCounts> {
    let mut counts = BTreeMap::<String, TabCounts>::new();
    for file in index.iter() {
        let names = match kind {
            LabelKind::Tags => &file.tags,
            LabelKind::Artists => &file.artists,
        };
        let popups = MediaTab::Popups.includes(file);
        let audio = MediaTab::Audio.includes(file);
        for name in names {
            let entry = counts.entry(name.clone()).or_default();
            entry.all += 1;
            entry.popups += popups as usize;
            entry.audio += audio as usize;
        }
    }
    counts
}
