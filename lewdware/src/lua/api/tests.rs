use shared::theme::Theme;

use crate::monitor::Monitor;

use super::*;

fn monitor() -> Monitor {
    Monitor {
        id: 1,
        primary: true,
        width: 1000,
        height: 800,
        scale_factor: 1.0,
    }
}

fn resolve_with(opts: SpawnWindowOpts) -> Result<PopupSpawnOpts, InvalidWindowSize> {
    // A fixed, platform-independent pair, so that a test saying nothing about chrome gets the
    // same answer on every machine. Tests about the user's own setting name one explicitly.
    resolve_with_chrome(
        opts,
        ChromeDefaults {
            theme: ThemeChoice::Plain,
            appearance: AppearanceChoice::Light,
        },
    )
}

fn resolve_with_chrome(
    opts: SpawnWindowOpts,
    chrome: ChromeDefaults,
) -> Result<PopupSpawnOpts, InvalidWindowSize> {
    PopupSpawnOpts::resolve(
        opts,
        WindowSizeBehaviour::UseDefaults {
            width: 200,
            height: 150,
        },
        &monitor(),
        chrome,
        false,
        false,
        false,
    )
}

/// The point of the user's setting: a mode that never mentions themes -- the common case, and
/// every mode written before themes existed -- draws in the look the user picked, with no
/// cooperation from the mode's author required.
#[test]
fn a_window_the_mode_did_not_theme_takes_the_users_choice() {
    let chrome = ChromeDefaults {
        theme: ThemeChoice::Redmond,
        appearance: AppearanceChoice::Dark,
    };
    let resolved = resolve_with_chrome(SpawnWindowOpts::default(), chrome).unwrap();

    assert_eq!(resolved.theme, Theme::Redmond);
    assert_eq!(resolved.appearance, AppearanceChoice::Dark);

    // And it is sized by that theme's metrics, not by the API default's.
    let (pad_x, pad_y) = Theme::Redmond.metrics().outer_padding();
    assert_eq!(resolved.outer_width, resolved.width + pad_x);
    assert_eq!(resolved.outer_height, resolved.height + pad_y);
}

/// The other half of that bargain: a mode that *does* name a look gets it, so a window built
/// to impersonate a specific OS still can, and so naming a theme remains the way to pin the
/// metrics a mode's own layout arithmetic depends on.
#[test]
fn a_theme_the_mode_named_beats_the_users_choice() {
    let chrome = ChromeDefaults {
        theme: ThemeChoice::Redmond,
        appearance: AppearanceChoice::Dark,
    };
    let opts = SpawnWindowOpts {
        theme: Some(ThemeChoice::Aqua),
        appearance: Some(AppearanceChoice::Light),
        ..Default::default()
    };
    let resolved = resolve_with_chrome(opts, chrome).unwrap();

    assert_eq!(resolved.theme, Theme::Aqua);
    assert_eq!(resolved.appearance, AppearanceChoice::Light);
}

/// Each axis falls back on its own: a mode fixing the look it draws in has not thereby said
/// anything about light or dark, and the user's answer to that still stands.
#[test]
fn the_two_axes_fall_back_independently() {
    let chrome = ChromeDefaults {
        theme: ThemeChoice::Redmond,
        appearance: AppearanceChoice::Dark,
    };
    let resolved = resolve_with_chrome(
        SpawnWindowOpts {
            theme: Some(ThemeChoice::Aqua),
            ..Default::default()
        },
        chrome,
    )
    .unwrap();

    assert_eq!(resolved.theme, Theme::Aqua);
    assert_eq!(resolved.appearance, AppearanceChoice::Dark);
}


/// Every name the config app can write must round-trip, or a user's saved choice silently
/// becomes someone else's.
#[test]
fn every_theme_name_reads_back_as_the_choice_it_names() {
    for &choice in ThemeChoice::ALL {
        assert_eq!(ThemeChoice::from_name(choice.name()), Some(choice));
    }
    for &choice in AppearanceChoice::ALL {
        assert_eq!(AppearanceChoice::from_name(choice.name()), Some(choice));
    }
}

#[test]
fn a_theme_is_resolved_before_the_window_is_sized() {
    // `native` is an alias; what reaches the window must already be a concrete look, and the
    // outer size must have been computed from *that* theme's metrics.
    let opts = SpawnWindowOpts {
        theme: Some(ThemeChoice::Native),
        ..Default::default()
    };
    let resolved = resolve_with(opts).unwrap();

    assert!(crate::window::theme::ALL_THEMES.contains(&resolved.theme));

    let (pad_x, pad_y) = resolved.theme.metrics().outer_padding();
    assert_eq!(resolved.outer_width, resolved.width + pad_x);
    assert_eq!(resolved.outer_height, resolved.height + pad_y);
}

/// Each theme's own metrics drive the outer size, which is the whole reason the theme has to
/// be known this early rather than at draw time.
#[test]
fn each_theme_sizes_its_window_by_its_own_metrics() {
    for &theme in crate::window::theme::ALL_THEMES {
        let choice = match theme {
            Theme::Plain => ThemeChoice::Plain,
            Theme::Fluent => ThemeChoice::Fluent,
            Theme::Redmond => ThemeChoice::Redmond,
            Theme::Aqua => ThemeChoice::Aqua,
            Theme::Adwaita => ThemeChoice::Adwaita,
            Theme::Breeze => ThemeChoice::Breeze,
            Theme::Platinum => ThemeChoice::Platinum,
            Theme::Cde => ThemeChoice::Cde,
        };

        let resolved = resolve_with(SpawnWindowOpts {
            theme: Some(choice),
            ..Default::default()
        })
        .unwrap();

        let (pad_x, pad_y) = theme.metrics().outer_padding();
        assert_eq!(resolved.theme, theme);
        assert_eq!(resolved.outer_width, resolved.width + pad_x, "{theme:?}");
        assert_eq!(resolved.outer_height, resolved.height + pad_y, "{theme:?}");
    }
}

#[test]
fn an_undecorated_window_has_no_padding_whatever_its_theme() {
    let resolved = resolve_with(SpawnWindowOpts {
        theme: Some(ThemeChoice::Redmond),
        decorations: false,
        ..Default::default()
    })
    .unwrap();

    assert_eq!(resolved.outer_width, resolved.width);
    assert_eq!(resolved.outer_height, resolved.height);
}

/// A zero-sized window has no content area, no drawable header and no close button, so asking
/// for one is a mistake worth reporting rather than a window that cannot be seen or dismissed.
#[test]
fn a_zero_or_negative_requested_size_is_rejected() {
    for (width, height, axis) in [
        (Some(Coord::Pixel(0)), None, "width"),
        (None, Some(Coord::Pixel(0)), "height"),
        (Some(Coord::Pixel(-10)), None, "width"),
        (None, Some(Coord::Pixel(-1)), "height"),
        // A percentage that rounds down to nothing is the same mistake, less obviously.
        (Some(Coord::Percent { percent: 0.0 }), None, "width"),
        (Some(Coord::Percent { percent: 0.04 }), None, "width"),
    ] {
        let error = resolve_with(SpawnWindowOpts {
            width,
            height,
            ..Default::default()
        })
        .expect_err("expected a rejection");

        assert_eq!(error.axis, axis);
        assert!(error.pixels <= 0);
    }
}

#[test]
fn a_positive_requested_size_is_accepted() {
    for (width, height) in [
        (Some(Coord::Pixel(1)), None),
        (None, Some(Coord::Pixel(1))),
        (Some(Coord::Percent { percent: 0.1 }), None),
        (Some(Coord::Pixel(400)), Some(Coord::Pixel(300))),
    ] {
        assert!(
            resolve_with(SpawnWindowOpts {
                width: width.clone(),
                height: height.clone(),
                ..Default::default()
            })
            .is_ok(),
            "{width:?} x {height:?} should be allowed"
        );
    }
}

/// Sizes the engine derives itself are not rejected: a mode that never asked for a size should
/// not have a spawn fail because a measurement came out small.
#[test]
fn an_engine_derived_size_is_never_rejected() {
    let resolved = PopupSpawnOpts::resolve(
        SpawnWindowOpts::default(),
        WindowSizeBehaviour::UseDefaults {
            width: 0,
            height: 0,
        },
        &monitor(),
        ChromeDefaults::default(),
        false,
        false,
        false,
    );

    assert!(resolved.is_ok());
}

/// The error text is what a mode author sees in `lw mode dev`, so it should name the axis and
/// the value rather than just failing.
#[test]
fn the_size_error_names_the_axis_and_value() {
    let error = resolve_with(SpawnWindowOpts {
        width: Some(Coord::Pixel(0)),
        ..Default::default()
    })
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("width"), "{message}");
    assert!(message.contains('0'), "{message}");
}

/// The Lua-facing spelling of the option, since a mode writes it as a string.
#[test]
fn the_theme_option_deserialises_from_its_lua_name() {
    let opts: SpawnWindowOpts =
        serde_json::from_str(r#"{"theme": "native-retro"}"#).expect("should deserialise");
    assert_eq!(opts.theme, Some(ThemeChoice::NativeRetro));

    // Absent stays absent, rather than collapsing into a default here: it is what tells
    // `resolve` to use the user's own setting instead of a look the mode chose.
    let opts: SpawnWindowOpts = serde_json::from_str("{}").expect("should deserialise");
    assert_eq!(opts.theme, None);
    assert_eq!(opts.appearance, None);
}

#[test]
fn draggable_defaults_to_false_and_resolves_when_enabled() {
    let defaults: SpawnWindowOpts =
        serde_json::from_str("{}").expect("should deserialise defaults");
    assert!(!defaults.draggable);

    let opts: SpawnWindowOpts =
        serde_json::from_str(r#"{"draggable": true}"#).expect("should deserialise option");
    assert!(resolve_with(opts).unwrap().draggable);
}

/// An unknown name is an error rather than a silent fallback: for a mode author that is a
/// typo worth surfacing. A mode passing on a name it got from somewhere less trustworthy
/// checks it against `lewdware.themes` first.
#[test]
fn an_unknown_theme_name_is_rejected() {
    let result: Result<SpawnWindowOpts, _> = serde_json::from_str(r#"{"theme": "win7"}"#);
    assert!(result.is_err());
}

#[test]
fn anchor_resolves_each_axis_independently() {
    // A 100x50 window at coordinate (200, 80) on each axis.
    let (coord, width, height) = (200, 100u32, 50u32);

    // Horizontal: left leaves x unadjusted, center subtracts half the width, right
    // subtracts the full width -- regardless of which vertical anchor it's paired with.
    assert_eq!(Anchor::TopLeft.resolve_x(coord, width), 200);
    assert_eq!(Anchor::CenterLeft.resolve_x(coord, width), 200);
    assert_eq!(Anchor::BottomLeft.resolve_x(coord, width), 200);

    assert_eq!(Anchor::TopCenter.resolve_x(coord, width), 150);
    assert_eq!(Anchor::Center.resolve_x(coord, width), 150);
    assert_eq!(Anchor::BottomCenter.resolve_x(coord, width), 150);

    assert_eq!(Anchor::TopRight.resolve_x(coord, width), 100);
    assert_eq!(Anchor::CenterRight.resolve_x(coord, width), 100);
    assert_eq!(Anchor::BottomRight.resolve_x(coord, width), 100);

    // Vertical: same shape, independent of which horizontal anchor it's paired with.
    assert_eq!(Anchor::TopLeft.resolve_y(coord, height), 200);
    assert_eq!(Anchor::TopCenter.resolve_y(coord, height), 200);
    assert_eq!(Anchor::TopRight.resolve_y(coord, height), 200);

    assert_eq!(Anchor::CenterLeft.resolve_y(coord, height), 175);
    assert_eq!(Anchor::Center.resolve_y(coord, height), 175);
    assert_eq!(Anchor::CenterRight.resolve_y(coord, height), 175);

    assert_eq!(Anchor::BottomLeft.resolve_y(coord, height), 150);
    assert_eq!(Anchor::BottomCenter.resolve_y(coord, height), 150);
    assert_eq!(Anchor::BottomRight.resolve_y(coord, height), 150);
}

#[test]
fn anchor_mixed_axes_combine_independently() {
    // "top-center" should behave like TopLeft on y but Center on x -- the actual point of
    // the 9-point grid over the old 3-point diagonal-only version.
    let (coord, size) = (200, 100u32);
    assert_eq!(
        Anchor::TopCenter.resolve_x(coord, size),
        Anchor::Center.resolve_x(coord, size)
    );
    assert_eq!(
        Anchor::TopCenter.resolve_y(coord, size),
        Anchor::TopLeft.resolve_y(coord, size)
    );
}

#[test]
fn anchor_serializes_to_documented_strings() {
    let pairs = [
        (Anchor::TopLeft, "top-left"),
        (Anchor::TopCenter, "top-center"),
        (Anchor::TopRight, "top-right"),
        (Anchor::CenterLeft, "center-left"),
        (Anchor::Center, "center"),
        (Anchor::CenterRight, "center-right"),
        (Anchor::BottomLeft, "bottom-left"),
        (Anchor::BottomCenter, "bottom-center"),
        (Anchor::BottomRight, "bottom-right"),
    ];

    for (anchor, expected) in pairs {
        let json = serde_json::to_string(&anchor).unwrap();
        assert_eq!(json, format!("\"{expected}\""));
        let parsed: Anchor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, anchor);
    }
}
