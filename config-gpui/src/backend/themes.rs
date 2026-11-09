//! The window looks on offer. Ported from `config/src-tauri/src/commands/theme.rs`.
//!
//! Read straight from `shared::theme`, which is where the engine reads them too, so the picker
//! can never offer a look the engine cannot draw or show one in the wrong colours.

use shared::theme::{Appearance, Chrome, Metrics, Theme, ThemeChoice, ThemeInfo, Widgets};

/// Everything needed to draw one look in the picker.
#[derive(Clone, Copy)]
pub struct Look {
    pub metrics: Metrics,
    pub chrome: Chrome,
    pub widgets: Widgets,
}

impl Look {
    fn of(theme: Theme, appearance: Appearance) -> Self {
        Self {
            metrics: theme.metrics(),
            chrome: theme.chrome(appearance),
            widgets: *theme.widgets(appearance),
        }
    }
}

pub struct ThemeEntry {
    /// What `AppConfig::theme` stores. An alias keeps its own name: picking the merged card means
    /// "follow this machine", not "pin whatever it happens to be today".
    pub choice: ThemeChoice,
    /// An alias borrows the label of what it resolves to here, since that is what is shown.
    pub label: &'static str,
    pub supports_dark: bool,
    /// True for `native`/`native-retro`.
    pub matches_system: bool,
    /// The concrete look an alias stands for here, so a config pinning that look directly still
    /// lights up the merged card.
    pub resolves_to: Option<ThemeChoice>,
    pub light: Look,
    pub dark: Look,
}

impl ThemeEntry {
    pub fn is_selected(&self, current: ThemeChoice) -> bool {
        self.choice == current || self.resolves_to == Some(current)
    }

    pub fn look(&self, appearance: Appearance) -> &Look {
        match appearance {
            Appearance::Light => &self.light,
            Appearance::Dark => &self.dark,
        }
    }
}

fn resolve(info: &ThemeInfo) -> Theme {
    ThemeChoice::from_name(info.name)
        .expect("the catalogue and ThemeChoice agree -- pinned by a test in shared")
        .resolve()
}

pub fn catalogue() -> Vec<ThemeEntry> {
    // What the aliases come out as here. Those *are* those looks on this machine, so the concrete
    // ones are dropped below and the alias wears their label instead of showing one card twice.
    let aliased: Vec<&'static str> = shared::theme::THEMES
        .iter()
        .filter(|info| info.is_alias)
        .map(|info| resolve(info).name())
        .collect();

    shared::theme::THEMES
        .iter()
        .filter(|info| info.is_alias || !aliased.contains(&resolve(info).name()))
        .map(|info| {
            let theme = resolve(info);
            let label = if info.is_alias {
                shared::theme::theme(theme.name()).map_or(info.label, |resolved| resolved.label)
            } else {
                info.label
            };
            ThemeEntry {
                choice: ThemeChoice::from_name(info.name).expect("checked by resolve"),
                label,
                supports_dark: theme.supports_dark(),
                matches_system: info.is_alias,
                resolves_to: info
                    .is_alias
                    .then(|| ThemeChoice::from_name(theme.name()))
                    .flatten(),
                light: Look::of(theme, Appearance::Light),
                dark: Look::of(theme, Appearance::Dark),
            }
        })
        .collect()
}
