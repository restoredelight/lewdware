use crate::dto::{theme_look, ThemeCatalogueDto, ThemeEntryDto};

/// The window looks the user can choose between, and what each one looks like.
///
/// Straight from `shared::theme`, which is where the engine reads them too -- so the picker can
/// never offer a look the engine cannot draw, show one in the wrong colours, or fall behind a
/// theme added since. A command rather than a constant baked into the frontend for exactly that
/// reason: one source of truth, with the agreement between the catalogue and the drawable set
/// pinned by a test.
#[tauri::command]
pub fn get_theme_catalogue(window: tauri::WebviewWindow) -> ThemeCatalogueDto {
    use shared::theme::{Appearance, ThemeChoice, ThemeInfo};

    let resolve = |info: &ThemeInfo| {
        ThemeChoice::from_name(info.name)
            .expect("the catalogue and ThemeChoice agree -- pinned by a test in shared")
            .resolve()
    };

    // What the aliases come out as here. On this machine those *are* those looks, so offering both
    // would be the same card twice under two names -- the concrete ones are dropped below and the
    // alias wears their label instead.
    let aliased: Vec<&'static str> = shared::theme::THEMES
        .iter()
        .filter(|info| info.is_alias)
        .map(|info| resolve(info).name())
        .collect();

    let themes = shared::theme::THEMES
        .iter()
        .filter(|info| info.is_alias || !aliased.contains(&resolve(info).name()))
        .map(|info| {
            let theme = resolve(info);
            let label = if info.is_alias {
                // Whatever it resolved to, named plainly. "Match my system" told the user nothing
                // about what they were looking at.
                shared::theme::theme(theme.name()).map_or(info.label, |resolved| resolved.label)
            } else {
                info.label
            };

            ThemeEntryDto {
                name: info.name,
                label,
                supports_dark: theme.supports_dark(),
                matches_system: info.is_alias,
                resolves_to: info.is_alias.then(|| theme.name()),
                light: theme_look(theme, Appearance::Light),
                dark: theme_look(theme, Appearance::Dark),
            }
        })
        .collect();

    #[cfg(target_os = "linux")]
    let system_appearance = {
        let _ = window;
        shared::theme::system_appearance()
    };

    #[cfg(not(target_os = "linux"))]
    let system_appearance = window.theme().ok().map(|theme| match theme {
        tauri::Theme::Light => Appearance::Light,
        tauri::Theme::Dark => Appearance::Dark,
        _ => Appearance::Light,
    });

    ThemeCatalogueDto {
        themes,
        appearances: shared::theme::APPEARANCES.to_vec(),
        system_appearance,
    }
}
