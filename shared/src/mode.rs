mod header;
mod metadata;
mod read;

pub use header::{Header, VERSION_MAJOR, VERSION_MINOR};
pub use metadata::{
    ConditionValue, Metadata, ModeEntry, ModeGroup, ModeOption, OptionType, OptionValue,
    Permission, ShowWhen, SourceFile, resolve_options,
};
pub use read::{read_mode_metadata, read_mode_metadata_async, read_source_file};
