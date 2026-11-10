//! Icons: Lucide, through gpui-kit. gpui-kit embeds only its components' own ~100 icons by default,
//! so the extra ones the apps use are listed in [`ExtraIcons`] and served by [`AppAssets`], which
//! falls back to gpui-kit's bundle. Using an icon not listed here (or in gpui-kit's
//! `default-icons.txt`) renders nothing, so add it to the list first.
//!
//! The Svelte apps use Heroicons; the nearest Lucide glyph stands in for each.

use std::borrow::Cow;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, SharedString};

gpui_kit::assets::icon_assets!(
    pub ExtraIcons,
    [
        Brush,
        Clock,
        CodeXml,
        Image,
        LayoutGrid,
        Merge,
        Music,
        Pencil,
        Repeat,
        RotateCcw,
        Tag,
        Trash,
        Upload,
        Volume2,
        VolumeX,
    ]
);

/// The app's asset source: [`ExtraIcons`], then gpui-kit's bundle.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// An icon sized to the surrounding text and drawn in its color.
pub fn icon(name: IconName) -> Icon {
    Icon::new(name)
}
