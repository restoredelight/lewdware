//! Converts Edgeware/Edgeware++ packs (dir or zip, modern `index.json` or legacy
//! `captions.json`/`media.json`/`prompt.json`/`web.json` layouts) into the pieces a `.lwpack`
//! needs: a tagged media list, pack metadata, and a behaviour.json `Content` section. See
//! `behaviour-design/edgeware-compat.md` for the full design.
//!
//! Scoped to **content only** -- `corruption.json` -> timeline and `config.json` -> frequency
//! anchors are a later milestone; this crate only notes their presence via a warning.
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
