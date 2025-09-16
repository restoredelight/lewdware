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
