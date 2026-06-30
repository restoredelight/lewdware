mod header;
mod metadata;
mod read;

pub use header::{Header, VERSION_MAJOR, VERSION_MINOR};
pub use metadata::{
    ConditionValue, Metadata, Mode, ModeEntry, ModeGroup, ModeOption, OptionType, OptionValue,
    ShowWhen, SourceFile,
};
pub use read::{read_mode_metadata, read_mode_metadata_async, read_source_file};
