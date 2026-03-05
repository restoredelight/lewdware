mod header;
mod metadata;
mod read;

pub use header::Header;
pub use metadata::{Metadata, Mode, ModeOption, OptionType, OptionValue, SourceFile};
pub use read::{read_mode_metadata, read_mode_metadata_async, read_source_file};
