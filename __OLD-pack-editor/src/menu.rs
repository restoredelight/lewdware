use dioxus_desktop::muda::{Menu, MenuItem, Result, Submenu, accelerator::{Accelerator, Modifiers, Code}};

pub enum MenuAction {
    New,
    Open,
    Save,
    SaveAs,
}

impl MenuAction {
    fn text(&self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Open => "Open",
            Self::Save => "Save",
            Self::SaveAs => "Save As...",
        }
    }
}

impl std::fmt::Display for MenuAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New => write!(f, "new"),
            Self::Open => write!(f, "open"),
            Self::Save => write!(f, "save"),
            Self::SaveAs => write!(f, "save-as"),
        }
    }
}

#[derive(Debug)]
pub struct InvalidMenuAction;

impl std::fmt::Display for InvalidMenuAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid menu action")
    }
}

impl std::error::Error for InvalidMenuAction {}

impl std::str::FromStr for MenuAction {
    type Err = InvalidMenuAction;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "new" => Self::New,
            "open" => Self::Open,
            "save" => Self::Save,
            "save-as" => Self::SaveAs,
            _ => return Err(InvalidMenuAction),
        })
    }
}

pub fn create_menu() -> Result<Menu> {
    Menu::with_items(&[&Submenu::with_items(
        "File",
        true,
        &[
            &MenuItem::with_id(
                MenuAction::New.to_string(),
                MenuAction::New.text(),
                true,
                None,
            ),
            &MenuItem::with_id(
                MenuAction::Open.to_string(),
                MenuAction::Open.text(),
                true,
                Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyO)),
            ),
            &MenuItem::with_id(
                MenuAction::Save.to_string(),
                MenuAction::Save.text(),
                true,
                Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyS)),
            ),
            &MenuItem::with_id(
                MenuAction::SaveAs.to_string(),
                MenuAction::SaveAs.text(),
                true,
                Some(Accelerator::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS)),
            ),
        ],
    )?])
}
