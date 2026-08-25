//! The behaviour document, as typed queries and one command per author action.
//!
//! Two rules shape this surface:
//!
//! - **A query serves a view.** Each returns what one surface renders, so the Content tab does not
//!   read the timeline to count its captions.
//! - **A mutation sets one field**, addressed by the id of the thing it changes, and is one
//!   transaction and one undo entry. Where one click legitimately changes several columns — the
//!   mitosis toggle sets a chance *and* a count — that is one command, because the unit is the
//!   author's action rather than the database's column. See `design/editor-data-flow.md`.
//!
//! Sending whole entities is what this replaced. Two edits to different fields of one entity used
//! to collide unless the front end accumulated them first, and managing that accumulation produced
//! three bugs in as many rounds. Per-field writes commute, so there is nothing to accumulate.
//!
//! `label` is the author's word for what they did — it becomes the undo entry, so it is UI copy and
//! belongs to the front end. It addresses nothing.

pub mod attributes;
pub mod group;
pub mod link;
pub mod pool;
pub mod queries;
pub mod stage;
pub mod timeline;
pub mod transition;

/// Defines one field-setting command.
///
/// Every one of them is the same twelve lines around a different call into
/// `shared::behaviour::editor` — take the pack, open one transaction, record one undo entry, report
/// nothing back. Written out forty times that shape is impossible to scan and easy to get subtly
/// wrong; here the only thing that varies is on the page.
///
/// The generated function's name *is* the name the front end invokes, so it stays greppable from
/// both ends. Tauri offers a `rename`, but it does not scope: `#[tauri::command]` emits
/// `__cmd__<fn>` at the crate root, so two modules cannot both hold a `set_tags` whatever the wire
/// name says. Names are therefore unique and descriptive, and `api.ts` does the grouping.
macro_rules! setter {
    (
        $(#[$meta:meta])*
        $name:ident($($arg:ident: $ty:ty),* $(,)?) |$tx:ident| $body:expr
    ) => {
        $(#[$meta])*
        #[tauri::command]
        pub async fn $name(
            state: tauri::State<'_, $crate::AppState>,
            $($arg: $ty,)*
            label: String,
        ) -> Result<(), String> {
            state
                .with_pack(async |pack| {
                    pack.behaviour_edit($crate::pack::BehaviourAction::new(
                        label,
                        move |$tx: &rusqlite::Transaction<'_>| $body,
                    ))
                    .await
                    .map(|_| ())
                })
                .await
        }
    };
}

pub(crate) use setter;
