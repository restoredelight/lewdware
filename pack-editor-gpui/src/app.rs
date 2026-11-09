//! The window's root: the start screen (new, open, recent packs, import from Edgeware) or the
//! open pack's editor, and the lifecycle between them -- opening, recovering unsaved work, saving,
//! saving as, discarding changes, closing, and asking first when closing would lose work. Also
//! the window title, and the file pickers that start an import. Ported from
//! `pack-editor/src/lib/Start.svelte`, `Editor.svelte`, `packActions` and
//! `src-tauri/src/commands/{lifecycle,upload}.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context as _};
use futures::{FutureExt, StreamExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    actions, div, px, AsyncApp, Context, Entity, FocusHandle, FontWeight, SharedString, WeakEntity,
    Window,
};
use pack_editor_core::pack::MediaPack;
use pack_editor_core::recents::{self, RecentPack};
use pack_editor_core::session::lifecycle::{
    create_from_edgeware, ConversionWarning, EdgewareMedia,
};
use pack_editor_core::session::Session;
use uuid::Uuid;

use crate::editor::{Editor, SaveProgress};
use crate::shell::Shell;
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, Size, Variant};
use crate::{keep_awake, runtime};

actions!(
    pack_editor,
    [
        Undo,
        Redo,
        Save,
        SaveAs,
        ClosePack,
        DiscardChanges,
        /// Start screen: a new draft.
        NewPack,
        /// Start screen: pick a pack to open.
        OpenPack,
        /// Pick files to import into the open pack. Views can dispatch it (e.g. an empty
        /// media list's "Import files..." button).
        ImportFiles,
        /// Pick a folder to import into the open pack.
        ImportFolder,
    ]
);

pub const APP_TITLE: &str = "Lewdware pack editor";

/// The key context the editor's shortcuts are bound in (see `main.rs`).
pub const KEY_CONTEXT: &str = "PackEditor";

enum Screen {
    Start,
    /// Opening or creating a pack; the message says which.
    Busy(SharedString),
    /// A saved pack opened with work from an earlier session that was never saved: keep it or
    /// throw it away, before the editor shows it.
    Recovered(Box<Session>),
    Editor {
        editor: Entity<Editor>,
        shell: Entity<Shell>,
    },
}

/// What to open.
enum Source {
    New,
    Pick,
    Path(PathBuf),
    Draft(Uuid),
    /// Pick an Edgeware `.zip` and convert it into a new draft.
    PickEdgeware,
    Edgeware(PathBuf),
}

/// An opened pack, and what has to happen once the editor is showing it.
struct Opened {
    session: Session,
    /// An Edgeware import: the media still to bring in, and what didn't convert.
    edgeware: Option<(EdgewareMedia, Vec<ConversionWarning>)>,
    /// A saved pack with unsaved work recovered from an earlier session.
    recovered: bool,
}

/// What closing is for: back to the start screen, or quitting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseIntent {
    Pack,
    Window,
}

pub struct Root {
    screen: Screen,
    recents: Vec<RecentPack>,
    /// A failure from the start screen (opening, creating). The editor shows its own.
    error: Option<SharedString>,
    /// Closing is waiting on the unsaved-changes dialog.
    confirm_close: Option<CloseIntent>,
    /// "Discard changes" is waiting on its confirmation.
    confirm_discard: bool,
    /// What the window title was last set to.
    title: String,
    focus: FocusHandle,
    dialog_focus: FocusHandle,
}

impl Root {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let root = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            root.update(cx, |root, cx| root.should_close_window(window, cx))
                .unwrap_or(true)
        });

        let mut this = Self {
            screen: Screen::Start,
            recents: Vec::new(),
            error: None,
            confirm_close: None,
            confirm_discard: false,
            title: APP_TITLE.to_string(),
            focus,
            dialog_focus: cx.focus_handle(),
        };
        this.refresh_recents(cx);
        // The headless check can't drive the file picker, so debug builds can be pointed at an
        // Edgeware zip to convert at startup instead.
        if cfg!(debug_assertions) {
            if let Some(path) = std::env::var_os("PACK_EDITOR_CHECK_EDGEWARE") {
                cx.defer_in(window, move |this, window, cx| {
                    this.open(Source::Edgeware(PathBuf::from(path)), window, cx)
                });
            }
        }
        this
    }

    fn editor(&self) -> Option<&Entity<Editor>> {
        match &self.screen {
            Screen::Editor { editor, .. } => Some(editor),
            _ => None,
        }
    }

    /// Has the views commit the field being typed in, then runs `then`, which sees the edit.
    fn after_commit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if let Some(editor) = self.editor() {
            editor.update(cx, |editor, cx| editor.commit_pending(cx));
        }
        cx.defer_in(window, then);
    }

    fn refresh_recents(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let recents = runtime::blocking(recents::list).await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.recents = recents;
                cx.notify();
            });
        })
        .detach();
    }

    // ── Opening ─────────────────────────────────────────────────────────────────────────────

    fn open(&mut self, source: Source, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Start) {
            return;
        }
        self.error = None;
        if !matches!(source, Source::Pick | Source::PickEdgeware) {
            self.screen = Screen::Busy(busy_message(&source));
        }
        cx.notify();
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let opened = open_session(source, &this, cx).await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    match opened {
                        Ok(Some(opened)) if opened.recovered => {
                            this.screen = Screen::Recovered(Box::new(opened.session));
                            window.focus(&this.dialog_focus, cx);
                        }
                        Ok(Some(opened)) => this.show_editor(opened, window, cx),
                        Ok(None) => this.screen = Screen::Start,
                        Err(error) => {
                            tracing::warn!("couldn't open a pack: {error:#}");
                            this.screen = Screen::Start;
                            this.error = Some(format!("{error:#}").into());
                            this.refresh_recents(cx);
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn show_editor(&mut self, opened: Opened, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.new_editor(opened.session, cx);
        if let Some((media, warnings)) = opened.edgeware {
            editor.update(cx, |editor, cx| editor.import_edgeware(media, warnings, cx));
        }
        // The headless check can't drop files on the window, so debug builds can be told what to
        // import as a pack opens (paths separated as in PATH).
        if cfg!(debug_assertions) {
            if let Some(paths) = std::env::var_os("PACK_EDITOR_CHECK_IMPORT") {
                let paths = std::env::split_paths(&paths).collect();
                editor.update(cx, |editor, cx| editor.import_paths(paths, cx));
            }
        }
        let shell = cx.new(|cx| Shell::new(editor.clone(), window, cx));
        self.screen = Screen::Editor { editor, shell };
        window.focus(&self.focus, cx);
    }

    fn new_editor(&mut self, session: Session, cx: &mut Context<Self>) -> Entity<Editor> {
        let editor = cx.new(|_| Editor::new(session));
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        editor
    }

    /// Swaps in a fresh editor for the pack the shell is showing (Save As, discarding changes),
    /// keeping the view it was on.
    fn replace_editor(&mut self, session: Session, cx: &mut Context<Self>) {
        let editor = self.new_editor(session, cx);
        if let Screen::Editor { shell, .. } = &self.screen {
            let shell = shell.clone();
            shell.update(cx, |shell, cx| shell.set_editor(editor.clone(), cx));
            self.screen = Screen::Editor { editor, shell };
        }
    }

    // ── Recovered work ──────────────────────────────────────────────────────────────────────

    /// Answers the "Unsaved changes found" dialog: keep the recovered work, throw it away, or
    /// (`None`) not open the pack after all.
    fn resolve_recovered(
        &mut self,
        keep: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Screen::Recovered(session) = std::mem::replace(&mut self.screen, Screen::Start) else {
            return;
        };
        let session = *session;
        match keep {
            Some(true) => {
                let opened = Opened {
                    session,
                    edgeware: None,
                    recovered: false,
                };
                self.show_editor(opened, window, cx);
            }
            Some(false) => {
                self.screen = Screen::Busy("Discarding changes...".into());
                let pack = session.pack().clone();
                drop(session);
                let window_handle = window.window_handle();
                cx.spawn(async move |this, cx| {
                    let reopened = discard_and_reopen(pack).await;
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            match reopened {
                                Ok(session) => {
                                    let opened = Opened {
                                        session,
                                        edgeware: None,
                                        recovered: false,
                                    };
                                    this.show_editor(opened, window, cx);
                                }
                                Err(error) => {
                                    this.screen = Screen::Start;
                                    this.error = Some(
                                        format!("Couldn't discard the changes: {error:#}").into(),
                                    );
                                }
                            }
                            cx.notify();
                        });
                    });
                })
                .detach();
            }
            None => {
                // Closing keeps the recovered work on disk for next time.
                let pack = session.pack().clone();
                drop(session);
                runtime::spawn(async move { pack.close().await });
                window.focus(&self.focus, cx);
            }
        }
        cx.notify();
    }

    // ── Discarding ──────────────────────────────────────────────────────────────────────────

    fn request_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor().cloned() else {
            return;
        };
        let (busy, importing, can_discard) = {
            let editor = editor.read(cx);
            (
                editor.saving.is_some(),
                editor.importing(),
                editor.is_dirty() && !editor.pack().is_untitled(),
            )
        };
        if busy || !can_discard {
            return;
        }
        if importing {
            editor.update(cx, |editor, cx| {
                editor.report("Stop the import before discarding changes.", cx)
            });
            return;
        }
        self.confirm_discard = true;
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    /// Restores the pack from its file, throwing away everything since the last save.
    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard = false;
        window.focus(&self.focus, cx);
        let Some(editor) = self.editor().cloned() else {
            return;
        };
        let (pack, stop) = {
            let editor = editor.read(cx);
            (editor.pack().clone(), editor.imports().stop())
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let reopened = async {
                runtime::run(stop).await?;
                discard_and_reopen(pack).await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                match reopened {
                    Ok(session) => this.replace_editor(session, cx),
                    Err(error) => {
                        tracing::warn!("couldn't discard changes: {error:#}");
                        editor.update(cx, |editor, cx| {
                            editor.report(format!("Couldn't discard the changes: {error:#}"), cx)
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ── Importing ───────────────────────────────────────────────────────────────────────────

    /// Picks files (or a folder) and imports them into the open pack.
    fn pick_import(&mut self, folder: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.editor().cloned() else {
            return;
        };
        if editor.read(cx).saving.is_some() {
            return;
        }
        cx.spawn(async move |_, cx| {
            let picked = runtime::run(async move {
                let dialog = rfd::AsyncFileDialog::new();
                if folder {
                    dialog
                        .set_title("Import a folder")
                        .pick_folder()
                        .await
                        .map(|folder| vec![folder.path().to_path_buf()])
                } else {
                    dialog
                        .set_title("Import files")
                        .pick_files()
                        .await
                        .map(|files| {
                            files
                                .into_iter()
                                .map(|file| file.path().to_path_buf())
                                .collect()
                        })
                }
            })
            .await;
            match picked {
                Ok(Some(paths)) => {
                    editor.update(cx, |editor, cx| editor.import_paths(paths, cx));
                }
                Ok(None) => {}
                Err(error) => {
                    editor.update(cx, |editor, cx| {
                        editor.report(format!("Couldn't open the file picker: {error:#}"), cx)
                    });
                }
            }
        })
        .detach();
    }

    // ── Saving ──────────────────────────────────────────────────────────────────────────────

    fn save(&mut self, save_as: bool, then_close: Option<CloseIntent>, cx: &mut Context<Self>) {
        let Some(editor) = self.editor().cloned() else {
            return;
        };
        if editor.read(cx).saving.is_some() {
            return;
        }
        let pack = editor.read(cx).pack().clone();
        let imports_done = editor.read(cx).imports().exclusive();
        cx.spawn(async move |this, cx| {
            let saved = save_pack(pack, save_as, imports_done, &editor, cx).await;
            let _ = this.update(cx, |this, cx| {
                match saved {
                    Ok(Saved::Cancelled) => return,
                    Ok(Saved::InPlace) => {}
                    // Save As opened the copy: edits from here on go to it.
                    Ok(Saved::As(session)) => this.replace_editor(*session, cx),
                    Err(error) => {
                        tracing::warn!("couldn't save: {error:#}");
                        if let Some(editor) = this.editor() {
                            editor.update(cx, |editor, cx| {
                                editor.error = Some(format!("Couldn't save: {error:#}").into());
                                cx.notify();
                            });
                        }
                        return;
                    }
                }
                if let Some(intent) = then_close {
                    this.close(false, intent, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ── Closing ─────────────────────────────────────────────────────────────────────────────

    fn request_close(&mut self, intent: CloseIntent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else {
            if intent == CloseIntent::Window {
                cx.quit();
            }
            return;
        };
        if editor.read(cx).saving.is_some() {
            return;
        }
        if editor.read(cx).importing() {
            editor.update(cx, |editor, cx| {
                editor.report("Stop the import before closing the pack.", cx)
            });
            return;
        }
        if editor.read(cx).is_dirty() {
            self.confirm_close = Some(intent);
            window.focus(&self.dialog_focus, cx);
            cx.notify();
        } else {
            self.close(false, intent, cx);
        }
    }

    fn should_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Screen::Recovered(_) = self.screen {
            // Not opened yet: closing it keeps the recovered work for next time.
            self.resolve_recovered(None, window, cx);
            return true;
        }
        if self.editor().is_none() {
            return true;
        }
        // Closing the pack properly (and asking first if it has unsaved work) quits afterwards.
        self.after_commit(window, cx, |this, window, cx| {
            this.request_close(CloseIntent::Window, window, cx)
        });
        false
    }

    /// Leaves the editor. `discard` throws away unsaved work (and a never-saved draft entirely).
    fn close(&mut self, discard: bool, intent: CloseIntent, cx: &mut Context<Self>) {
        self.confirm_close = None;
        let Some(editor) = self.editor().cloned() else {
            return;
        };
        let pack = editor.read(cx).pack().clone();
        let imports_closed = editor.read(cx).imports().close();
        // The editor goes first, and its `Session` with it, so nothing writes while the pack shuts.
        self.screen = Screen::Busy("Closing...".into());
        drop(editor);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let closed = runtime::run(async move {
                imports_closed.await;
                let draft = pack.is_untitled().then(|| pack.id());
                let saved = pack.is_saved().await;
                if discard {
                    pack.close_and_discard().await;
                } else {
                    pack.close().await;
                }
                // A draft leaves the recent list when it's discarded, or when it was saved
                // elsewhere and this copy has nothing new.
                if let Some(id) = draft.filter(|_| discard || saved) {
                    let _ = recents::forget_draft(id);
                }
            })
            .await;
            if let Err(error) = closed {
                tracing::warn!("closing the pack failed: {error:#}");
            }
            let _ = this.update(cx, |this, cx| {
                if intent == CloseIntent::Window {
                    cx.quit();
                    return;
                }
                this.screen = Screen::Start;
                this.refresh_recents(cx);
                cx.notify();
            });
        })
        .detach();
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    fn start_screen(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let new = entity.clone();
        let pick = entity.clone();
        div()
            .size_full()
            .flex()
            .justify_center()
            .overflow_hidden()
            .child(
                div()
                    .w(px(560.))
                    .max_w_full()
                    .mt(px(96.))
                    .px_6()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(ui::page_title("Pack editor"))
                            .child(ui::hint(
                                "Packs hold the images, videos and audio Lewdware shows, and how it uses them.",
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("new-pack", "New pack")
                                    .variant(Variant::Primary)
                                    .on_click(move |_, window, cx| {
                                        new.update(cx, |this, cx| this.open(Source::New, window, cx))
                                    }),
                            )
                            .child(
                                Button::new("open-pack", "Open pack...")
                                    .variant(Variant::Secondary)
                                    .on_click(move |_, window, cx| {
                                        pick.update(cx, |this, cx| {
                                            this.open(Source::Pick, window, cx)
                                        })
                                    }),
                            ),
                    )
                    .children(self.error.clone().map(|error| {
                        div()
                            .px_3()
                            .py_2()
                            .rounded(theme::RADIUS_MD)
                            .border_1()
                            .border_color(c(theme::DANGER_BORDER))
                            .bg(c(theme::DANGER_BG))
                            .text_color(c(theme::DANGER))
                            .text_size(px(13.))
                            .child(error)
                    }))
                    .child(self.recent_list(cx))
                    .child(self.edgeware_section(cx)),
            )
    }

    fn edgeware_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let import = cx.entity();
        ui::card().child(
            div()
                .px_3()
                .py_3()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(ui::label("Moving from Edgeware?"))
                        .child(ui::hint(
                            "Import an Edgeware pack and convert it into an editable Lewdware pack.",
                        )),
                )
                .child(
                    Button::new("import-edgeware", "Import Edgeware pack...")
                        .variant(Variant::Secondary)
                        .size(Size::Compact)
                        .on_click(move |_, window, cx| {
                            import.update(cx, |this, cx| {
                                this.open(Source::PickEdgeware, window, cx)
                            })
                        }),
                ),
        )
    }

    fn recent_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.recents.iter().enumerate().map(|(index, recent)| {
            let open = cx.entity();
            let remove = cx.entity();
            let source = match (&recent.path, &recent.draft_id) {
                (Some(path), _) => Some(Source::Path(PathBuf::from(path))),
                (None, Some(id)) => Uuid::parse_str(id).ok().map(Source::Draft),
                (None, None) => None,
            };
            let detail: SharedString = match &recent.path {
                Some(path) => path.clone().into(),
                None => "Draft, never saved".into(),
            };
            let path = recent.path.clone();
            let draft = recent
                .draft_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok());
            let source = std::cell::RefCell::new(source);
            div()
                .id(("recent", index))
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .cursor_pointer()
                .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                .on_click(move |_, window, cx| {
                    if let Some(source) = source.borrow_mut().take() {
                        open.update(cx, |this, cx| this.open(source, window, cx));
                    }
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(recent.name.clone()),
                        )
                        .child(ui::readout(detail).truncate()),
                )
                .child(
                    Button::new(("remove-recent", index), "Remove")
                        .variant(Variant::Quiet)
                        .size(Size::Compact)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let path = path.clone();
                            remove.update(cx, |this, cx| this.remove_recent(path, draft, cx));
                        }),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(ui::section_title("Recent"))
            .child(if self.recents.is_empty() {
                ui::hint("Packs you open or create appear here.").into_any_element()
            } else {
                ui::card().children(rows).into_any_element()
            })
    }

    fn remove_recent(&mut self, path: Option<String>, draft: Option<Uuid>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let removed = runtime::blocking(move || recents::remove(path.as_deref(), draft)).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = removed.and_then(|r| r) {
                    this.error = Some(format!("Couldn't remove that: {error:#}").into());
                }
                this.refresh_recents(cx);
            });
        })
        .detach();
    }

    fn close_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let intent = self.confirm_close?;
        let name = self.editor()?.read(cx).pack().name();
        let cancel = cx.entity();
        let discard = cx.entity();
        let save = cx.entity();
        let close = cx.entity();
        Some(ui::dialog(
            "Unsaved changes",
            format!("Save the changes to \"{name}\" before closing?"),
            vec![
                DialogAction::new("Cancel", Variant::Secondary, move |_, _, cx| {
                    cancel.update(cx, |this, cx| {
                        this.confirm_close = None;
                        cx.notify();
                    })
                }),
                DialogAction::new("Discard", Variant::Destructive, move |_, _, cx| {
                    discard.update(cx, |this, cx| this.close(true, intent, cx))
                }),
                DialogAction::new("Save", Variant::Primary, move |_, _, cx| {
                    save.update(cx, |this, cx| {
                        this.confirm_close = None;
                        this.save(false, Some(intent), cx);
                    })
                }),
            ],
            move |_, cx| {
                close.update(cx, |this, cx| {
                    this.confirm_close = None;
                    cx.notify();
                })
            },
            &self.dialog_focus,
            window,
        ))
    }

    fn recovered_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let Screen::Recovered(session) = &self.screen else {
            return None;
        };
        let name = session.pack().name();
        let (cancel, discard, keep, close) = (cx.entity(), cx.entity(), cx.entity(), cx.entity());
        Some(ui::dialog(
            "Unsaved changes found",
            format!(
                "\"{name}\" has changes from an earlier session that were never saved. Keep \
                 them, or go back to the last saved version?"
            ),
            vec![
                DialogAction::new("Cancel", Variant::Secondary, move |_, window, cx| {
                    cancel.update(cx, |this, cx| this.resolve_recovered(None, window, cx))
                }),
                DialogAction::new(
                    "Discard changes",
                    Variant::Destructive,
                    move |_, window, cx| {
                        discard.update(cx, |this, cx| {
                            this.resolve_recovered(Some(false), window, cx)
                        })
                    },
                ),
                DialogAction::new("Keep changes", Variant::Primary, move |_, window, cx| {
                    keep.update(cx, |this, cx| {
                        this.resolve_recovered(Some(true), window, cx)
                    })
                }),
            ],
            move |window, cx| close.update(cx, |this, cx| this.resolve_recovered(None, window, cx)),
            &self.dialog_focus,
            window,
        ))
    }

    fn discard_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        if !self.confirm_discard {
            return None;
        }
        self.editor()?;
        let (cancel, discard, close) = (cx.entity(), cx.entity(), cx.entity());
        let dismiss = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.confirm_discard = false;
            window.focus(&this.focus, cx);
            cx.notify();
        };
        Some(ui::dialog(
            "Discard changes?",
            "Everything since the pack was last saved will be lost. This can't be undone.",
            vec![
                DialogAction::new("Cancel", Variant::Secondary, move |_, window, cx| {
                    cancel.update(cx, |this, cx| dismiss(this, window, cx))
                }),
                DialogAction::new(
                    "Discard changes",
                    Variant::Destructive,
                    move |_, window, cx| discard.update(cx, |this, cx| this.discard(window, cx)),
                ),
            ],
            move |window, cx| close.update(cx, |this, cx| dismiss(this, window, cx)),
            &self.dialog_focus,
            window,
        ))
    }

    /// "Name - Lewdware pack editor", with a leading `*` while there's unsaved work.
    fn update_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = match self.editor() {
            Some(editor) => {
                let editor = editor.read(cx);
                window_title(&editor.pack().name(), editor.is_dirty())
            }
            None => APP_TITLE.to_string(),
        };
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.screen {
            Screen::Start => self.start_screen(cx).into_any_element(),
            Screen::Busy(message) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::readout(message.clone()))
                .into_any_element(),
            Screen::Recovered(_) => div().size_full().into_any_element(),
            Screen::Editor { shell, .. } => shell.clone().into_any_element(),
        };
        self.update_title(window, cx);
        let dialog = self.close_dialog(window, cx);
        let recovered = self.recovered_dialog(window, cx);
        let discard = self.discard_dialog(window, cx);

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                if let Some(editor) = this.editor() {
                    editor.update(cx, |editor, cx| editor.undo(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &Redo, _, cx| {
                if let Some(editor) = this.editor() {
                    editor.update(cx, |editor, cx| editor.redo(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &Save, window, cx| {
                this.after_commit(window, cx, |this, _, cx| this.save(false, None, cx))
            }))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| {
                this.after_commit(window, cx, |this, _, cx| this.save(true, None, cx))
            }))
            .on_action(cx.listener(|this, _: &ClosePack, window, cx| {
                this.after_commit(window, cx, |this, window, cx| {
                    this.request_close(CloseIntent::Pack, window, cx)
                })
            }))
            .on_action(
                cx.listener(|this, _: &DiscardChanges, window, cx| {
                    this.request_discard(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ImportFiles, _, cx| this.pick_import(false, cx)))
            .on_action(cx.listener(|this, _: &ImportFolder, _, cx| this.pick_import(true, cx)))
            .on_action(
                cx.listener(|this, _: &NewPack, window, cx| this.open(Source::New, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenPack, window, cx| this.open(Source::Pick, window, cx)),
            )
            .relative()
            .size_full()
            .bg(c(theme::BG))
            .text_color(c(theme::TEXT))
            .text_size(px(13.))
            .font_family(theme::FONT_SANS)
            .child(body)
            .children(dialog)
            .children(recovered)
            .children(discard)
    }
}

fn window_title(name: &str, dirty: bool) -> String {
    format!("{}{name} - {APP_TITLE}", if dirty { "*" } else { "" })
}

fn busy_message(source: &Source) -> SharedString {
    match source {
        Source::New => "Creating a pack...".into(),
        Source::Edgeware(_) | Source::PickEdgeware => "Converting the Edgeware pack...".into(),
        _ => "Opening pack...".into(),
    }
}

/// Restores `pack` from its file and indexes it again.
async fn discard_and_reopen(pack: Arc<MediaPack>) -> anyhow::Result<Session> {
    let restored = pack.clone();
    runtime::run(async move { restored.discard_changes().await }).await??;
    runtime::blocking(move || Session::open(pack))
        .await?
        .context("Couldn't read the pack's media")
}

/// Opens or creates the pack and its session. `Ok(None)` when the author cancelled the picker.
async fn open_session(
    source: Source,
    this: &WeakEntity<Root>,
    cx: &mut AsyncApp,
) -> anyhow::Result<Option<Opened>> {
    let data_dir = dirs::data_dir().ok_or_else(|| anyhow!("Couldn't find the data directory"))?;
    let source = match source {
        Source::Pick | Source::PickEdgeware => {
            let edgeware = matches!(source, Source::PickEdgeware);
            let picked = runtime::run(async move {
                let dialog = rfd::AsyncFileDialog::new();
                let dialog = if edgeware {
                    dialog
                        .set_title("Select an Edgeware pack (.zip)")
                        .add_filter("Zip archive", &["zip"])
                } else {
                    dialog
                        .set_title("Open pack")
                        .add_filter("Lewdware pack", &["lwpack"])
                };
                dialog
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            })
            .await?;
            let Some(path) = picked else {
                return Ok(None);
            };
            let source = if edgeware {
                Source::Edgeware(path)
            } else {
                Source::Path(path)
            };
            // Choosing isn't opening: no message until there's something to open.
            let message = busy_message(&source);
            let _ = this.update(cx, |this, cx| {
                this.screen = Screen::Busy(message);
                cx.notify();
            });
            source
        }
        source => source,
    };

    let from_file = matches!(source, Source::Path(_));
    let (pack, edgeware) = runtime::run(async move {
        let (pack, edgeware) = match source {
            Source::New => (
                MediaPack::new_unsaved(&data_dir, "Untitled Pack").await?,
                None,
            ),
            Source::Path(path) => (MediaPack::open(path, &data_dir).await?, None),
            Source::Draft(id) => (MediaPack::recover_unsaved(&data_dir, id).await?, None),
            Source::Edgeware(path) => {
                let draft = create_from_edgeware(&path, &data_dir)
                    .await
                    .context("Couldn't import the Edgeware pack")?;
                (draft.pack, Some((draft.media, draft.warnings)))
            }
            Source::Pick | Source::PickEdgeware => unreachable!("resolved to a path above"),
        };
        match pack.path() {
            Some(path) => recents::remember_pack(path, pack.name())?,
            None => recents::remember_draft(pack.id(), pack.name())?,
        }
        anyhow::Ok((pack, edgeware))
    })
    .await??;

    // A saved pack with an unsaved working copy left over from an earlier session.
    let recovered = from_file && !pack.is_saved_now();
    let session = runtime::blocking(move || Session::open(Arc::new(pack)))
        .await?
        .context("Couldn't read the pack's media")?;
    Ok(Some(Opened {
        session,
        edgeware,
        recovered,
    }))
}

enum Saved {
    Cancelled,
    InPlace,
    /// Saved to a new file, which is now the open pack.
    As(Box<Session>),
}

/// Saves the open pack, asking for a destination when it has none (or `save_as`). Progress is
/// shown on the editor while it runs.
async fn save_pack<Guard: Send + 'static>(
    pack: Arc<MediaPack>,
    save_as: bool,
    imports_done: impl std::future::Future<Output = Guard> + Send + 'static,
    editor: &Entity<Editor>,
    cx: &mut AsyncApp,
) -> anyhow::Result<Saved> {
    let destination = if save_as || pack.path().is_none() {
        let suggested = format!("{}.lwpack", sanitize_file_name(&pack.name()));
        let title = if save_as { "Save pack as" } else { "Save pack" };
        let picked = runtime::run(async move {
            rfd::AsyncFileDialog::new()
                .set_title(title)
                .add_filter("Lewdware pack", &["lwpack"])
                .set_file_name(suggested)
                .save_file()
                .await
                .map(|file| file.path().to_path_buf())
        })
        .await?;
        match picked {
            Some(path) => Some(path),
            None => return Ok(Saved::Cancelled),
        }
    } else {
        None
    };

    let set_progress = |progress: Option<SaveProgress>, cx: &mut AsyncApp| {
        editor.update(cx, |editor, cx| {
            editor.saving = progress;
            cx.notify();
        });
    };
    set_progress(Some(SaveProgress::default()), cx);

    // Saving somewhere new moves the working copy, so a running import finishes first (and no
    // new one starts until the save is done).
    let _imports = match destination {
        Some(_) => match runtime::run(imports_done).await {
            Ok(guard) => Some(guard),
            Err(error) => {
                set_progress(None, cx);
                return Err(error);
            }
        },
        None => None,
    };

    let (sender, mut progress) = futures::channel::mpsc::unbounded::<SaveProgress>();
    let on_progress = move |saved, total| {
        let _ = sender.unbounded_send(SaveProgress { saved, total });
    };
    let work = {
        let pack = pack.clone();
        let destination = destination.clone();
        runtime::run(async move {
            let _awake = keep_awake::acquire("Saving a pack").await;
            match destination {
                Some(path) => pack.save_as(&path, on_progress).await,
                None => pack
                    .save_with_in_place_notification(on_progress, || {})
                    .await
                    .map(|()| None),
            }
        })
    };
    let mut work = Box::pin(work).fuse();
    let saved = loop {
        futures::select_biased! {
            result = work => break result,
            update = progress.next() => {
                if let Some(update) = update {
                    set_progress(Some(update), cx);
                }
            }
        }
    };
    set_progress(None, cx);
    let new_pack = saved??;

    let Some(new_pack) = new_pack else {
        if let Some(path) = pack.path() {
            recents::remember_pack(path, pack.name())?;
        }
        return Ok(Saved::InPlace);
    };

    let was_untitled = pack.is_untitled();
    if was_untitled {
        recents::forget_draft(pack.id())?;
    }
    if let Some(path) = new_pack.path() {
        recents::remember_pack(path, new_pack.name())?;
    }
    let session = runtime::blocking(move || Session::open(Arc::new(new_pack))).await??;
    if !was_untitled {
        // The old file stays as it was; its working copy is done with.
        runtime::run(async move { pack.close().await }).await?;
    }
    Ok(Saved::As(Box::new(session)))
}

/// A pack name as a file name: Windows' reserved characters (the strictest set) become `_`.
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    match cleaned.trim() {
        "" => "Untitled Pack".to_string(),
        trimmed => trimmed.to_string(),
    }
}
