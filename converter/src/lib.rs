//! Converts Edgeware/Edgeware++ packs (dir or zip, modern `index.json` or legacy
//! `captions.json`/`media.json`/`prompt.json`/`web.json` layouts) into the pieces a `.lwpack`
//! needs: a tagged media list, pack metadata, and a behaviour.json `Content` + `Experience`
//! section (`corruption.json` -> transition timeline, `config.json` -> frequency anchors). See
//! `behaviour-design/edgeware-compat.md` for the full design.
//!
//! ```no_run
//! use converter::{DirSource, convert};
//!
//! let source = DirSource::new("/path/to/edgeware/pack");
//! let output = convert(&source);
//! for warning in &output.warnings {
//!     println!("{:?}: {}", warning.kind, warning.message);
//! }
//! ```

mod convert;
mod model;
mod parse;
mod slug;
mod source;

pub use convert::{ConversionOutput, ConvertedMedia, convert};
pub use model::{Warning, WarningKind};
pub use source::{DirSource, PackSource, ZipSource};
