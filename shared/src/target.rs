//! A bunch of generic/serde magic to represent certain repeated patterns in the config file
//! without writing custom serialization methods.
//!
//! Take, for example, the `notifications` field. Users can either pass in:
//!
//! - A single notification, e.g.:
//!   ``` json5
//!   notifications: "A notification"
//!   ```
//!
//! - Multiple notifications, e.g.:
//!   ``` json5
//!   notifications: ["Notification 1", "Notification 2"]
//!   ```
//!
//! - A notification with extra options, e.g.:
//!   ``` json5
//!   notifications: {
//!       summary: "My pack says:"
//!       text: "Hello!",
//!       tags: ["tag1", "tag2"]
//!   }
//!   ```
//! - Multiple notifications (using one or more of the above formats):
//!   ```json5
//!   notifications: [
//!       "My notification",
//!       {
//!           summary: "Hello",
//!           text: "How are you?"
//!       },
//!       {
//!           text: "Another notification",
//!           tags: ["tag2"]
//!       }
//!   ]
//!   ```
//!
//! - An object specifying options which apply to all notifications:
//!   ```json5
//!   notifications: {
//!       default: {
//!           summary: "Default summary",
//!           tags: ["notification"]
//!       },
//!       items: [
//!           "My notification",
//!           {
//!               summary: "I'm overriding the default summary!",
//!               text: "Another notification"
//!           }
//!       ]
//!   }
//!   ```
//!
//! We use an almost identical pattern for the `popups`, `links`, `prompts` and `wallpaper` fields.
//! Since this pattern is complex, we only want to write code handling all these cases once. Hence
//! the `Target` struct, which attempts to be generic over this pattern.
//!
//! For examples of code that is generic over the `Target` struct, see `read.rs`.

use std::default;

use merge::Merge;
use serde::{Deserialize, Serialize};

/// An abstract type for representing objects of type:
/// | Item
/// | [ Item ]
/// | {
///     default: { ..Opts, tags },
///     ..ExtraOpts,
///     items: Item | [ Item ]
///   }
///
/// Where `Item` is:
/// | Primary // (usually a string)
/// | {
///     [PrimaryStruct.field]: Primary,
///     ..Opts,
///     tags
///   }
pub type Target<Primary, PrimaryStruct, Opts = Empty, ExtraOpts = Empty> = Either<Items<Primary, PrimaryStruct, Opts>, WithDefaults<Primary, PrimaryStruct, Opts, ExtraOpts>>;

/// A helper macro to create a struct to pass into the `PrimaryStruct` argument of `Target`.
#[macro_export]
macro_rules! create_arg {
    ($name:ident, $field:ident, $type:ty) => {
        #[derive(Serialize, Deserialize, Clone)]
        pub struct $name {
            $field: $type,
        }

        impl From<$name> for $type {
            fn from(value: $name) -> Self {
                value.$field
            }
        }
    };
}

#[derive(Serialize, Deserialize)]
pub struct WithDefaults<Primary, PrimaryStruct, Opts, ExtraOpts>
where
    Primary: Clone,
    Opts: default::Default + Merge + Clone,
    ExtraOpts: Clone,
    PrimaryStruct: Into<Primary>,
{
    #[serde(flatten)]
    pub extra_opts: ExtraOpts,
    #[serde(default)]
    pub default: Default<Opts>,
    pub items: Items<Primary, PrimaryStruct, Opts>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Default<Opts> where Opts: default::Default + Merge {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub opts: Opts,
}

#[derive(Serialize, Deserialize, Default, Clone, Merge)]
pub struct Empty {}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Items<Primary, PrimaryStruct, Opts> where Opts: default::Default {
    Single(Item<Primary, PrimaryStruct, Opts>),
    Multiple(Vec<Item<Primary, PrimaryStruct, Opts>>),
}

pub type Item<Primary, PrimaryStruct, Opts> = Either<Primary, FullItem<PrimaryStruct, Opts>>;

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Either<T, V> {
    Left(T),
    Right(V),
}

#[derive(Serialize, Deserialize)]
pub struct FullItem<PrimaryStruct, Opts> {
    #[serde(flatten)]
    pub primary: PrimaryStruct,
    #[serde(flatten)]
    pub opts: Opts,
    #[serde(default)]
    pub tags: Vec<String>,
}
