//! The small value types a theme is written in: a colour, a text alignment, and a typeface.
//!
//! These began in the engine, beside the Lua API that also uses them. They moved here with the
//! themes: a theme names a colour and a face in almost every field, so a description of one
//! cannot be read outside the engine unless these can be too. The engine re-exports `Color` and
//! `TextAlign` from its Lua layer, which is where its own code still refers to them.

use serde::{Deserialize, Serialize};

/// A typeface the engine can draw with.
///
/// Wider than the Lua-facing `TextFont`, which every author-set style uses: themes also need the
/// UI faces of the platforms they imitate, and those are not offered as text styles.
///
/// Only ever a *name* here. The files themselves are embedded by whoever draws — the engine from
/// the repo-level `assets/fonts/`, `config/` as `@font-face` rules over those same files — so
/// this stays a description, like the rest of a theme.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Face {
    /// egui's own proportional font, and what a header draws with unless a theme says otherwise.
    Default,
    /// egui's own monospace font.
    Mono,
    /// Anton — bold and high-impact, for emphasis.
    Display,
    /// W95FA — the Windows 95 UI face, for `redmond`.
    Pixel,
    /// Selawik — Microsoft's metrically-compatible substitute for Segoe UI, for `fluent`.
    Selawik,
    /// Inter — the closest freely-licensed stand-in for San Francisco, for `aqua`.
    Inter,
    /// Cantarell — GNOME's own UI font, for `adwaita`.
    Cantarell,
    /// Noto Sans — KDE Plasma's default UI face, for `breeze`.
    NotoSans,
    /// Liberation Sans — metrically compatible with Helvetica, for CDE/Motif widgets.
    LiberationSans,
    /// Liberation Sans Bold — CDE/Motif's sturdy active-title companion.
    LiberationSansBold,
    /// Source Sans 3 Regular — compact UI face used as a redistributable Charcoal stand-in.
    SourceSans,
    /// Source Sans 3 Semibold — the sturdier title-bar companion for `platinum`.
    SourceSansSemibold,
}

/// Where text sits within the space it is given.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let r = (self.r * 255.0).round() as u8;
        let g = (self.g * 255.0).round() as u8;
        let b = (self.b * 255.0).round() as u8;
        let a = (self.a * 255.0).round() as u8;
        if a == 255 {
            serializer.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}"))
        } else {
            serializer.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let hex = s
            .strip_prefix('#')
            .ok_or_else(|| serde::de::Error::custom("color must start with '#'"))?;

        fn channel(s: &str) -> Option<f32> {
            u8::from_str_radix(s, 16).ok().map(|v| v as f32 / 255.0)
        }

        match hex.len() {
            6 => Ok(Color {
                r: channel(&hex[0..2])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                g: channel(&hex[2..4])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                b: channel(&hex[4..6])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                a: 1.0,
            }),
            8 => Ok(Color {
                r: channel(&hex[0..2])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                g: channel(&hex[2..4])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                b: channel(&hex[4..6])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                a: channel(&hex[6..8])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
            }),
            _ => Err(serde::de::Error::custom(
                "color must be '#rrggbb' or '#rrggbbaa'",
            )),
        }
    }
}
