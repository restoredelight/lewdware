use std::collections::HashMap;

use serde::Serialize;
use serde_json::{Map, Value};

/// One Edgeware mood's (or the pack's un-moodded "default" bucket's) content pools -- mirrors
/// `EdgewarePlusPlus/edgeware/src/pack/data.py`'s `MoodBase` dataclass, since that's the real,
/// tested semantics both the modern and legacy formats converge to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MoodBase {
    pub captions: Vec<String>,
    /// Captured only so a non-empty pool earns a warning -- denial/blur is excluded from
    /// compat v1 (needs engine work), see `behaviour-design/edgeware-compat.md`.
    pub denial: Vec<String>,
    pub subliminals: Vec<String>,
    pub notifications: Vec<String>,
    pub prompts: Vec<String>,
    pub web: Vec<WebEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebEntry {
    pub url: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgewareMood {
    pub name: String,
    pub base: MoodBase,
}

/// `Default`-only fields, with no Python-style backfill: `None` unless the pack's own JSON
/// explicitly set them. Needed because both fields fold into behaviour.json fields
/// (`Content.captions` / `PromptSettings.submit_label`) that must stay unset -- not Edgeware's
/// own boilerplate ("I Submit <3") -- when the pack author never touched them, per the schema's
/// "no defaults injection" principle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DefaultExtra {
    pub popup_close: Option<String>,
    pub prompt_submit: Option<String>,
}

/// The unified internal model both the modern (`index.json`) and legacy (`captions.json` +
/// `media.json` + `prompt.json` + `web.json`) parsers produce, so everything downstream of
/// parsing (content assembly, content groups, media tagging) is format-agnostic.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EdgewareIndex {
    pub default: MoodBase,
    pub default_extra: DefaultExtra,
    pub moods: Vec<EdgewareMood>,
    /// media file name (as it appears under `img/`/`vid/`/`aud/`) -> mood name. `"default"` is
    /// never a key here -- files under Edgeware's "default" media bucket are mood-less, not
    /// tagged `"default"` (matches `load_media`'s `if mood != "default"` filter).
    pub media_moods: HashMap<String, String>,
}

impl EdgewareIndex {
    /// Finds or lazily creates a mood by name (mirrors Python's `get_or_add_mood`), returning
    /// its index in `self.moods`. Legacy parsing calls this once per prefix/mood key it meets
    /// across the four legacy files, so a mood referenced only by (say) `prompt.json` still gets
    /// a `ContentGroup` even though it tags no media.
    pub fn mood_index(&mut self, name: &str) -> usize {
        if let Some(i) = self.moods.iter().position(|m| m.name == name) {
            return i;
        }
        self.moods.push(EdgewareMood {
            name: name.to_string(),
            base: MoodBase::default(),
        });
        self.moods.len() - 1
    }
}

/// One numbered level from `corruption.json`, mirroring `EdgewarePlusPlus/edgeware/src/pack/
/// data.py`'s `CorruptionLevel` dataclass and `pack/load.py`'s `load_corruption` (numeric-string
/// keys `"1".."N"`, `"default"` wallpaper fallback for level 1 only). `added_moods`/
/// `removed_moods` are this level's own delta -- `convert.rs` folds them into a cumulative active
/// set while building the timeline, mirroring `apply_corruption_level`'s
/// `pack.active_moods.update(...).difference_update(...)`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CorruptionLevel {
    pub added_moods: Vec<String>,
    pub removed_moods: Vec<String>,
    pub wallpaper: Option<String>,
    /// This level's raw `config` override, if any (e.g. Edgeware's `{"promptMod": 10}`). Unlike
    /// the old per-level scalar-modifier schema, a `Level` now carries full `FrequencyAnchors`
    /// directly, so `convert.rs` can actually apply the keys it recognizes (see
    /// `LEVEL_ANCHOR_CONFIG_KEYS`) as real per-level anchor changes, folded cumulatively across
    /// levels the same way `added_moods`/`removed_moods` are -- rather than only ever warning
    /// about the drop. Any key it doesn't recognize (e.g. `promptMistakes`) still warns and drops.
    pub config: Map<String, Value>,
}

/// Structured category for a `Warning`, so a front end can group/count without parsing
/// `message`. See `behaviour-design/edgeware-compat.md`: "warnings are part of the API".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WarningKind {
    /// A JSON file was present but failed to parse; treated as absent, not fatal.
    MalformedSource,
    /// Real Edgeware behavior with no behaviour.json equivalent (denial captions, per-mood
    /// click/weight tuning) -- dropped, not converted.
    UnsupportedFeatureDropped,
    /// `script.lua` present -- Edgeware's custom Lua-subset scripting isn't converted.
    ScriptSkipped,
    /// `discord.dat` present -- Discord rich presence isn't supported.
    DiscordSkipped,
    /// `corruption.json` present but converted to zero usable timeline levels (e.g. empty
    /// `moods`/`wallpapers`/`config` sections).
    CorruptionNotConverted,
    /// `config.json` present with keys that have no Lewdware equivalent (left over once the
    /// recognized chance-per-tick pacing keys were mapped to `experience.anchors`).
    ConfigNotConverted,
    /// A file referenced by the pack's JSON (e.g. in `media_moods`) doesn't actually exist in
    /// the source.
    UnreadableMediaFile,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    pub kind: WarningKind,
    pub message: String,
}

impl Warning {
    pub fn new(kind: WarningKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mood_index_reuses_existing_mood() {
        let mut index = EdgewareIndex::default();
        let a = index.mood_index("vanilla");
        let b = index.mood_index("vanilla");
        assert_eq!(a, b);
        assert_eq!(index.moods.len(), 1);
    }

    #[test]
    fn mood_index_creates_new_moods_in_order() {
        let mut index = EdgewareIndex::default();
        assert_eq!(index.mood_index("vanilla"), 0);
        assert_eq!(index.mood_index("kinky"), 1);
        assert_eq!(index.moods[0].name, "vanilla");
        assert_eq!(index.moods[1].name, "kinky");
    }
}
