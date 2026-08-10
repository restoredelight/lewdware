use std::collections::HashMap;

use anyhow::Result;
use egui::TextEdit;
use tokio::sync::mpsc::UnboundedSender;
use winit::{
    dpi::PhysicalPosition,
    event::{Touch, WindowEvent},
};

use crate::lua::{self, DialogButton, DialogElement, DialogElementUpdate, ItemId, TextStyle};
use crate::media::ImageData;
use crate::text_font;
use crate::window::layer::bevel;
use crate::window::layer::egui_renderer::{EguiCPUWindow, EguiGpuRenderer};
use crate::window::layer::{CpuFrame, GpuFrame, LayerStatus};
use crate::window::state::WindowState;
use crate::window::surface::Buffer;
use crate::window::target::RenderTarget;
use crate::window::theme::{self, Theme, WidgetEdge, to_color32};

/// The render-time state of one dialog element. Image requirements have been uploaded to egui
/// textures, and input values are mutated in place as the user types.
enum DialogElementState {
    Text {
        id: Option<String>,
        text: String,
        style: TextStyle,
    },
    Image {
        id: Option<String>,
        texture: egui::TextureHandle,
        /// Retained so the texture can be re-uploaded into a fresh `Context` after a
        /// render-target fallback; egui offers no way to read pixels back out of a handle.
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    Input {
        id: String,
        placeholder: Option<String>,
        value: String,
    },
    Buttons {
        id: Option<String>,
        options: Vec<DialogButton>,
    },
}

impl DialogElementState {
    /// Re-upload an image element into a new egui `Context`. No-op for other element types.
    fn reload_texture(&mut self, context: &egui::Context, index: usize) {
        if let Self::Image {
            texture,
            width,
            height,
            pixels,
            ..
        } = self
        {
            *texture = upload_element_texture(context, index, *width, *height, pixels);
        }
    }

    fn id(&self) -> Option<&str> {
        match self {
            Self::Text { id, .. } | Self::Image { id, .. } | Self::Buttons { id, .. } => {
                id.as_deref()
            }
            Self::Input { id, .. } => Some(id),
        }
    }
}

/// What happened during one `render()` pass, resolved into a `lua::Event::DialogSelect`/
/// `DialogSubmit` once the element loop (which borrows `elements` mutably, to track live input
/// text) has finished — building the values snapshot needs a fresh immutable borrow.
enum DialogInteraction {
    Select(String),
    Submit(String),
}

/// The default button id, cached and recomputed whenever a `buttons` element's `options` change
/// (spawn time, and `update_element`) — pressing Enter in an input element clicks it instead of
/// firing `on_submit`. At most one across the whole dialog; if `update_element` is ever handed a
/// second one, the earlier button silently keeps the role rather than erroring (the strict check
/// only runs once, at spawn, in the Lua binding — see `spawn_dialog`).
fn find_default_button_id(elements: &[DialogElementState]) -> Option<String> {
    elements.iter().find_map(|element| match element {
        DialogElementState::Buttons { options, .. } => {
            options.iter().find(|o| o.default).map(|o| o.id.clone())
        }
        _ => None,
    })
}

fn collect_dialog_values(elements: &[DialogElementState]) -> HashMap<String, String> {
    elements
        .iter()
        .filter_map(|element| match element {
            DialogElementState::Input { id, value, .. } => Some((id.clone(), value.clone())),
            _ => None,
        })
        .collect()
}

fn send_dialog_interaction(
    lua_event_tx: &UnboundedSender<lua::Event>,
    id: ItemId,
    interaction: DialogInteraction,
    elements: &[DialogElementState],
) {
    let values = collect_dialog_values(elements);
    let event = match interaction {
        DialogInteraction::Select(button_id) => lua::Event::DialogSelect {
            id,
            button_id,
            values,
        },
        DialogInteraction::Submit(element_id) => lua::Event::DialogSubmit {
            id,
            element_id,
            values,
        },
    };
    if lua_event_tx.send(event).is_err() {
        tracing::debug!("Couldn't send dialog interaction event: Lua thread has shut down");
    }
}

/// Paint the dialog's elements as a vertical stack, in order. Returns the button click / input
/// submit that occurred this frame, if any (at most one is handled per frame — egui only reports
/// one click/submit per widget per frame anyway).
fn paint_dialog(
    ui: &mut egui::Ui,
    elements: &mut [DialogElementState],
    default_button_id: Option<&str>,
    edge: &WidgetEdge,
    default_button_style: theme::DefaultButtonStyle,
) -> Option<DialogInteraction> {
    let mut interaction = None;

    // A `CentralPanel` rather than a bare `Frame`, so the theme's panel fill covers the whole
    // window rather than only the strip its content happens to occupy — matching `paint_text`.
    // A frame on its own sizes to its contents, which left the rest of the window showing the
    // surface's clear colour.
    egui::CentralPanel::default()
        .frame(egui::Frame::central_panel(ui.style()).inner_margin(10.0))
        .show_inside(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                for element in elements.iter_mut() {
                    match element {
                        DialogElementState::Text { text, style, .. } => {
                            paint_dialog_text(ui, text, style);
                        }
                        DialogElementState::Image { texture, .. } => {
                            ui.add(
                                egui::Image::from_texture((texture.id(), texture.size_vec2()))
                                    .max_width(ui.available_width())
                                    .shrink_to_fit(),
                            );
                        }
                        DialogElementState::Input {
                            id,
                            placeholder,
                            value,
                        } => {
                            // egui sizes a `TextEdit` from its font, ignoring `interact_size`, so
                            // without this a field stays ~21pt tall next to a theme's 32pt buttons.
                            // Padded out to the theme's own control height instead.
                            let row = ui.text_style_height(&egui::TextStyle::Body);
                            let target = ui.spacing().interact_size.y;
                            let vertical = ((target - row) / 2.0).round().max(2.0);

                            let mut input = TextEdit::singleline(value)
                                .margin(egui::vec2(ui.spacing().button_padding.x, vertical));
                            if let Some(placeholder) = placeholder {
                                input = input.hint_text(placeholder.as_str());
                            }
                            let response = ui.add(input);
                            bevel::input_edge(ui, &response, edge);
                            if response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                interaction = Some(match default_button_id {
                                    Some(button_id) => {
                                        DialogInteraction::Select(button_id.to_string())
                                    }
                                    None => DialogInteraction::Submit(id.clone()),
                                });
                            }
                        }
                        DialogElementState::Buttons { options, .. } => {
                            // Allocated at the row's natural width, so the enclosing
                            // `top_down(Align::Center)` centres it under the centred text above.
                            // Clamped to what is available, which is also what makes wrapping
                            // kick in for a dialog with more buttons than fit on one line.
                            let width = button_row_width(ui, options).min(ui.available_width());

                            ui.allocate_ui_with_layout(
                                // Full remaining height, not zero: the region has to be at least
                                // as tall as the buttons that go in it, or egui (rightly) reports
                                // the content as escaping its own region. `Align::Min` below keeps
                                // the row at the top of it, and `allocate_ui_with_layout` advances
                                // the cursor by what was *used*, so claiming the rest costs
                                // nothing.
                                egui::vec2(width, ui.available_height()),
                                // `Align::Min` is the *cross* axis here: without it the row is
                                // centred vertically in all the height left in the dialog, which
                                // strands it far below the element above.
                                //
                                // And no `with_main_justify`: a wrapping layout reports an
                                // infinite main extent, so "fill the main axis" made each button
                                // infinitely wide — which drew its label at x = infinity, off
                                // screen, and swallowed every button after the first.
                                egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
                                |ui| {
                                    // No explicit spacing: egui already puts `item_spacing.x`
                                    // between widgets in a horizontal layout, and adding more on
                                    // top is what made `button_row_width` under-measure.
                                    for option in options.iter() {
                                        let style = (Some(option.id.as_str()) == default_button_id)
                                            .then_some(default_button_style);
                                        if bevel::button(ui, &option.label, edge, style).clicked() {
                                            interaction =
                                                Some(DialogInteraction::Select(option.id.clone()));
                                        }
                                    }
                                },
                            );
                        }
                    }
                    ui.add_space(10.0);
                }
            });
        });

    interaction
}

/// The width a row of buttons wants, laid out on one line.
///
/// Measured rather than asked for, because egui sizes a button from its label and there is no way
/// to query that without building one. Used only to decide how wide a region to allocate; the
/// buttons still size themselves.
fn button_row_width(ui: &egui::Ui, options: &[DialogButton]) -> f32 {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let padding = ui.spacing().button_padding.x * 2.0;
    let minimum = ui.spacing().interact_size.x;
    // What egui itself puts between widgets in a horizontal layout.
    let gap = ui.spacing().item_spacing.x;

    let buttons: f32 = options
        .iter()
        .map(|option| {
            let galley = ui.ctx().fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    option.label.clone(),
                    font_id.clone(),
                    egui::Color32::PLACEHOLDER,
                )
            });
            // Rounded *up*, per button: a total measured even a fraction of a pixel short of what
            // egui goes on to use makes the last button wrap onto a line of its own.
            (galley.size().x + padding).max(minimum).ceil()
        })
        .sum();

    (buttons + gap * options.len().saturating_sub(1) as f32).ceil()
}

/// Paint one `text` element inline within the dialog's vertical stack (unlike `paint_text`,
/// which centers within the whole window for a standalone text popup).
fn paint_dialog_text(ui: &mut egui::Ui, text: &str, style: &TextStyle) {
    let font_size = style.font_size.to_pixels(0);
    // `default` means the surrounding UI face inside a dialog. Explicit author faces still win;
    // standalone text popups retain their existing neutral default because they have no themed
    // controls to belong to.
    let family = dialog_text_family(ui, style.font);
    let font_id = egui::FontId::new(font_size, family);

    // Unset follows the theme, so a dialog's own text is readable in a dark palette rather than
    // black on near-black. The dialog owns its background, so it knows what suits it.
    let color = style
        .color
        .map(to_color32)
        .unwrap_or_else(|| ui.visuals().text_color());

    let outline_width = if style.outline_color.is_some() {
        style.outline_width
    } else {
        0.0
    };
    let wrap_width = (ui.available_width() - outline_width * 2.0).max(0.0);

    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id,
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = wrap_width;
    job.halign = text_font::to_egui_align(style.align);

    let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));

    let size = egui::vec2(ui.available_width(), galley.size().y + outline_width * 2.0);
    let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

    let pos_x = match style.align {
        lua::TextAlign::Left => rect.left() + outline_width,
        lua::TextAlign::Center => rect.center().x,
        lua::TextAlign::Right => rect.right() - outline_width,
    };
    let pos = egui::pos2(pos_x, rect.top() + outline_width);

    let painter = ui.painter();

    if let Some(outline_color) = style.outline_color {
        let outline_color = to_color32(outline_color);
        let count = text_font::outline_sample_count(style.outline_width);
        for offset in text_font::outline_offsets(count) {
            painter.galley_with_override_text_color(
                pos + offset * style.outline_width,
                galley.clone(),
                outline_color,
            );
        }
    }

    if style.bold {
        const BOLD_OFFSET: f32 = 0.6;
        for offset in text_font::outline_offsets(8) {
            painter.galley_with_override_text_color(
                pos + offset * BOLD_OFFSET,
                galley.clone(),
                color,
            );
        }
    }

    painter.galley_with_override_text_color(pos, galley, color);
}

fn dialog_text_family(ui: &egui::Ui, font: lua::TextFont) -> egui::FontFamily {
    if font == lua::TextFont::Default {
        egui::TextStyle::Body.resolve(ui.style()).family
    } else {
        text_font::font_family(font)
    }
}

/// Content painted with egui: either a dialog (interactive elements) or a static text popup.
/// The two share all of their plumbing and differ only in what they paint and which Lua
/// operations apply to them.
pub struct EguiLayer {
    paint: EguiPaint,
    backend: EguiBackend,
    /// Kept so the backend can be rebuilt after a render-target fallback.
    background_color: Option<lua::Color>,
    /// Resolved once, at construction: `draw_cpu` paints without access to the `WindowState` the
    /// theme lives on.
    edge: WidgetEdge,
    /// Persistent theme-specific mark for the action Enter activates.
    default_button_style: theme::DefaultButtonStyle,
    /// Set while painting, sent by [`EguiLayer::flush_events`] once the frame is done — building
    /// the values snapshot needs a fresh immutable borrow of `elements`, which the paint closure
    /// held mutably.
    pending: Option<DialogInteraction>,
}

/// Exactly one of these exists per layer, chosen from the render target it was built for.
enum EguiBackend {
    Gpu(Box<EguiGpuRenderer>),
    Cpu(Box<EguiCPUWindow>),
}

/// What an [`EguiLayer`] paints, and the state backing it.
///
/// A text popup has no interactive widgets, so (per egui's repaint-on-demand model) it only ever
/// redraws when `set_text()` is called or the window is first shown. A dialog repaints
/// continuously in response to input.
enum EguiPaint {
    Dialog {
        elements: Vec<DialogElementState>,
        default_button_id: Option<String>,
    },
    Text {
        text: String,
        style: TextStyle,
    },
}

impl EguiPaint {
    /// Paint one frame. Returns the button click / input submit that occurred, if any — always
    /// `None` for text content.
    fn run(
        &mut self,
        ui: &mut egui::Ui,
        edge: &WidgetEdge,
        default_button_style: theme::DefaultButtonStyle,
    ) -> Option<DialogInteraction> {
        match self {
            Self::Dialog {
                elements,
                default_button_id,
            } => paint_dialog(
                ui,
                elements,
                default_button_id.as_deref(),
                edge,
                default_button_style,
            ),
            Self::Text { text, style } => {
                paint_text(ui, text, style);
                None
            }
        }
    }

    /// The font set this content needs. A text popup picks its own font per popup; a dialog's
    /// widgets take the theme's, while its `text` elements still style themselves individually.
    fn font_definitions(&self, theme: Theme) -> Option<egui::FontDefinitions> {
        match self {
            Self::Dialog { .. } => theme::widget_font_definitions(theme),
            Self::Text { style, .. } => text_font::build_font_definitions(style.font),
        }
    }
}

fn build_backend(
    paint: &EguiPaint,
    state: &WindowState,
    target: &RenderTarget,
    background_color: Option<lua::Color>,
) -> Result<EguiBackend> {
    let theme = state.theme();
    let font_definitions = paint.font_definitions(theme);
    let style = theme::window_style(theme, state.appearance(), background_color);

    if target.is_gpu() {
        Ok(EguiBackend::Gpu(Box::new(EguiGpuRenderer::new(
            target.wgpu_state(),
            state.window(),
            state.inner_size(),
            state.opacity,
            target.premultiplied_alpha(),
            target.force_opaque(),
            style,
            font_definitions,
            state.redraw_requester(),
        )?)))
    } else {
        Ok(EguiBackend::Cpu(Box::new(EguiCPUWindow::new(
            state.window().clone(),
            style,
            font_definitions,
            state.redraw_requester(),
        )?)))
    }
}

impl EguiLayer {
    pub fn new_dialog<I>(
        state: &WindowState,
        target: &RenderTarget,
        elements: Vec<DialogElement<I>>,
        mut resolve_image: impl FnMut(I) -> Result<ImageData>,
    ) -> Result<Self> {
        let background_color = state.background_color();

        // Built with an empty element list so the egui `Context` exists to upload textures into.
        let mut paint = EguiPaint::Dialog {
            elements: Vec::new(),
            default_button_id: None,
        };
        let backend = build_backend(&paint, state, target, background_color)?;

        let loaded: Vec<_> = elements
            .into_iter()
            .enumerate()
            .map(|(index, element)| {
                load_element(&backend.context(), index, element, &mut resolve_image)
            })
            .collect::<Result<_>>()?;

        let EguiPaint::Dialog {
            elements,
            default_button_id,
        } = &mut paint
        else {
            unreachable!("just constructed as Dialog");
        };
        *default_button_id = find_default_button_id(&loaded);
        *elements = loaded;

        Ok(Self {
            paint,
            backend,
            background_color,
            edge: state.theme().widget_edge(state.appearance()),
            default_button_style: state.theme().default_button_style(state.appearance()),
            pending: None,
        })
    }

    pub fn new_text(
        state: &WindowState,
        target: &RenderTarget,
        text: String,
        style: TextStyle,
    ) -> Result<Self> {
        // Unlike a dialog (which falls back to egui's opaque light theme), an unset
        // `background_color` means a fully transparent panel for text windows.
        let background_color = Some(state.background_color().unwrap_or(lua::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        let paint = EguiPaint::Text { text, style };
        let backend = build_backend(&paint, state, target, background_color)?;

        Ok(Self {
            paint,
            backend,
            background_color,
            edge: state.theme().widget_edge(state.appearance()),
            default_button_style: state.theme().default_button_style(state.appearance()),
            pending: None,
        })
    }

    /// Rebuild the egui backend after the render target changed backend.
    ///
    /// egui's own widget state (focus, text cursor, scroll) does not survive this — it lives in
    /// the `Context` being replaced. That is acceptable for the only trigger, a wgpu device loss.
    pub fn rebuild(&mut self, state: &WindowState, target: &RenderTarget) -> Result<()> {
        self.backend = build_backend(&self.paint, state, target, self.background_color)?;

        // Dialog images live in the old `Context`'s texture set, so re-upload them.
        if let EguiPaint::Dialog { elements, .. } = &mut self.paint {
            let context = self.backend.context();
            for (index, element) in elements.iter_mut().enumerate() {
                element.reload_texture(&context, index);
            }
        }

        Ok(())
    }

    pub fn handle_event(&mut self, state: &WindowState, event: &WindowEvent) {
        let translated = if state.decorations().enabled() {
            Some(translate_event_position(
                event.clone(),
                state.inner_offset(),
            ))
        } else {
            None
        };
        let event = translated.as_ref().unwrap_or(event);

        match &mut self.backend {
            EguiBackend::Gpu(gpu) => gpu.handle_event(state.window(), event),
            EguiBackend::Cpu(cpu) => cpu.handle_event(event),
        }
    }

    /// egui runs its UI during `prepare`, not `draw`: on the GPU path the UI is rendered into an
    /// intermediate texture that `draw` then blits, and on the CPU path the interaction has to be
    /// resolved before the borrow of `elements` is released.
    pub fn prepare_gpu(
        &mut self,
        state: &WindowState,
        frame: &GpuFrame<'_>,
    ) -> Result<LayerStatus> {
        let EguiBackend::Gpu(gpu) = &mut self.backend else {
            return Ok(LayerStatus::Idle);
        };

        let window = state.window().clone();
        let inner_size = state.inner_size();
        let paint = &mut self.paint;
        let edge = self.edge;
        let default_button_style = self.default_button_style;

        let mut interaction = None;
        gpu.render_to_texture(frame.wgpu, &window, inner_size, |ui| {
            interaction = paint.run(ui, &edge, default_button_style);
        })?;
        gpu.set_opacity(&frame.wgpu.queue, frame.opacity);

        self.pending = interaction;

        Ok(LayerStatus::Draw)
    }

    pub fn draw_gpu(&self, rpass: &mut wgpu::RenderPass<'static>, frame: &GpuFrame<'_>) {
        let EguiBackend::Gpu(gpu) = &self.backend else {
            return;
        };

        rpass.set_pipeline(frame.pipeline);
        rpass.set_bind_group(0, &gpu.bind_group, &[]);
        rpass.set_bind_group(1, &gpu.window_bind_group, &[]);
        frame.content.set_viewport(rpass);
        rpass.draw(0..4, 0..1);
    }

    pub fn prepare_cpu(&mut self, _frame: &CpuFrame) -> Result<LayerStatus> {
        Ok(LayerStatus::Draw)
    }

    pub fn draw_cpu(&mut self, buffer: &mut Buffer, frame: &CpuFrame) {
        let EguiBackend::Cpu(cpu) = &mut self.backend else {
            return;
        };

        let content = frame.content;
        let mut pixels = vec![0u32; (content.width * content.height) as usize];
        let mut buffer_ref = egui_software_backend::BufferMutRef::new(
            bytemuck::cast_slice_mut(&mut pixels),
            content.width as usize,
            content.height as usize,
        );

        let paint = &mut self.paint;
        let edge = self.edge;
        let default_button_style = self.default_button_style;
        let mut interaction = None;
        let _ = cpu.redraw(&mut buffer_ref, |ui| {
            interaction = paint.run(ui, &edge, default_button_style);
        });

        buffer.copy_from_u32_buf(&pixels, content.width, content.x, content.y);

        self.pending = interaction;
    }

    /// Send any interaction recorded while painting. Called once the frame is complete.
    pub fn flush_events(&mut self, state: &WindowState) {
        let Some(interaction) = self.pending.take() else {
            return;
        };
        let EguiPaint::Dialog { elements, .. } = &self.paint else {
            return;
        };

        send_dialog_interaction(
            state.lua_event_tx(),
            state.popup_id(),
            interaction,
            elements,
        );
    }

    /// Replace a text popup's text. Returns `false` if this is a dialog.
    pub fn set_text(&mut self, text: String) -> bool {
        let EguiPaint::Text { text: current, .. } = &mut self.paint else {
            return false;
        };

        *current = text;

        true
    }

    /// Applies whichever fields in `props` are relevant to the target element's type, ignoring
    /// the rest (e.g. `options` on a `text` element is a no-op, not an error). Returns whether an
    /// element with `id` was found — `image` updates aren't supported yet (see
    /// `DialogElementUpdate`'s doc comment), so an `image` element is found but never changed.
    /// Also `false` if this is a text popup rather than a dialog.
    pub fn update_element(&mut self, id: &str, props: DialogElementUpdate) -> bool {
        let EguiPaint::Dialog {
            elements,
            default_button_id,
        } = &mut self.paint
        else {
            return false;
        };

        let Some(element) = elements.iter_mut().find(|e| e.id() == Some(id)) else {
            return false;
        };

        match element {
            DialogElementState::Text { text, style, .. } => {
                if let Some(new_text) = props.text {
                    *text = new_text;
                }
                if let Some(font) = props.font {
                    style.font = font;
                }
                if let Some(font_size) = props.font_size {
                    style.font_size = font_size;
                }
                if let Some(color) = props.color {
                    style.color = Some(color);
                }
                if let Some(bold) = props.bold {
                    style.bold = bold;
                }
                if let Some(align) = props.align {
                    style.align = align;
                }
                if let Some(outline_color) = props.outline_color {
                    style.outline_color = Some(outline_color);
                }
                if let Some(outline_width) = props.outline_width {
                    style.outline_width = outline_width;
                }
            }
            DialogElementState::Image { .. } => {}
            DialogElementState::Input {
                placeholder, value, ..
            } => {
                if let Some(new_placeholder) = props.placeholder {
                    *placeholder = Some(new_placeholder);
                }
                if let Some(new_value) = props.value {
                    *value = new_value;
                }
            }
            DialogElementState::Buttons { options, .. } => {
                if let Some(new_options) = props.options {
                    *options = new_options;
                }
            }
        }

        *default_button_id = find_default_button_id(elements);

        true
    }

    /// `None` if `id` doesn't name an `input` element (including if the window is closed, since
    /// the caller — `WindowRequestSender::get_dialog_value` — folds that case into the same
    /// `None` as `DialogWindow:values()`).
    pub fn value(&self, id: &str) -> Option<String> {
        let EguiPaint::Dialog { elements, .. } = &self.paint else {
            return None;
        };

        elements.iter().find_map(|element| match element {
            DialogElementState::Input {
                id: element_id,
                value,
                ..
            } if element_id == id => Some(value.clone()),
            _ => None,
        })
    }

    pub fn values(&self) -> HashMap<String, String> {
        let EguiPaint::Dialog { elements, .. } = &self.paint else {
            return HashMap::new();
        };

        collect_dialog_values(elements)
    }
}

impl EguiBackend {
    /// Cheap to clone -- `egui::Context` is an `Arc` handle internally.
    fn context(&self) -> egui::Context {
        match self {
            Self::Gpu(gpu) => gpu.context(),
            Self::Cpu(cpu) => cpu.context(),
        }
    }
}

/// Turn a resolved element spec into render-ready state, uploading an `image` element's
/// pixels as an egui texture (named uniquely per element, since texture names must be
/// distinct within a `Context`).
fn load_element<I>(
    context: &egui::Context,
    index: usize,
    element: DialogElement<I>,
    resolve_image: &mut impl FnMut(I) -> Result<ImageData>,
) -> Result<DialogElementState> {
    Ok(match element {
        DialogElement::Text { id, text, style } => DialogElementState::Text { id, text, style },
        DialogElement::Image { id, image } => {
            let data = resolve_image(image)?;
            let width = data.width();
            let height = data.height();
            let pixels = data.into_vec();
            let texture = upload_element_texture(context, index, width, height, &pixels);
            DialogElementState::Image {
                id,
                texture,
                width,
                height,
                pixels,
            }
        }
        DialogElement::Input {
            id,
            placeholder,
            initial_value,
        } => DialogElementState::Input {
            id,
            placeholder,
            value: initial_value.unwrap_or_default(),
        },
        DialogElement::Buttons { id, options } => DialogElementState::Buttons { id, options },
    })
}

fn upload_element_texture(
    context: &egui::Context,
    index: usize,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> egui::TextureHandle {
    let color_image =
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], pixels);
    context.load_texture(
        format!("dialog-image-{index}"),
        color_image,
        egui::TextureOptions::default(),
    )
}

/// Paint `text` styled by `style`, centred vertically and horizontally-aligned per
/// `style.align` within the available area. `outline_color`/`bold` are faked by repainting the
/// same laid-out `Galley` at small offsets before the crisp final draw, since egui has no native
/// stroked-text support.
fn paint_text(ui: &mut egui::Ui, text: &str, style: &TextStyle) {
    // Keep the panel's fill (driven by `background_color`, via `visuals.panel_fill`) but drop its
    // default margin: the window is sized to fit the text exactly (see `calculate_text_popup_size`),
    // so a margin here would make the available area smaller than what was measured, causing text
    // to wrap (or clip) when it shouldn't.
    let frame = egui::Frame::central_panel(ui.style()).inner_margin(0);

    egui::CentralPanel::default()
        .frame(frame)
        .show_inside(ui, |ui| {
            let available = ui.available_rect_before_wrap();
            // `style.font_size` is always `FontSize::Value` by the time it reaches here — percentage
            // sizes are resolved once in `App::spawn_text`, while the monitor is known — so the
            // argument here is unused.
            let font_size = style.font_size.to_pixels(0);
            let font_id = egui::FontId::new(font_size, text_font::font_family(style.font));
            // Unset stays black here, unlike a dialog: a text popup floats over the desktop with a
            // transparent background by default, so there is no surface to take a cue from — which
            // is what `outline_color` is for.
            let color = to_color32(style.color.unwrap_or(lua::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            }));

            // The window was sized (see `calculate_text_popup_size`) with `2 * outline_width` of
            // extra room baked in so the outline stroke (drawn offset from the text on every
            // side) doesn't get clipped by the window bounds. Mirror that padding here so
            // wrapping/positioning match what was actually measured.
            let outline_width = if style.outline_color.is_some() {
                style.outline_width
            } else {
                0.0
            };
            let wrap_width = (available.width() - outline_width * 2.0).max(0.0);

            let mut job = egui::text::LayoutJob::single_section(
                text.to_owned(),
                egui::TextFormat {
                    font_id,
                    color,
                    ..Default::default()
                },
            );
            job.wrap.max_width = wrap_width;
            job.halign = text_font::to_egui_align(style.align);

            let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));

            // `halign` positions each row *relative to x=0* (LEFT: [0,w], Center: [-w/2,w/2],
            // RIGHT: [-w,0]), not within `[0, wrap_width]` — so the anchor we paint at has to shift
            // depending on alignment, not just sit at the left edge. Left/right also inset by
            // `outline_width` so the outline has room on the outer edge (center is already
            // symmetric).
            let pos_x = match style.align {
                lua::TextAlign::Left => available.left() + outline_width,
                lua::TextAlign::Center => available.center().x,
                lua::TextAlign::Right => available.right() - outline_width,
            };
            let pos = egui::pos2(pos_x, available.center().y - galley.size().y / 2.0);

            let painter = ui.painter();

            if let Some(outline_color) = style.outline_color {
                let outline_color = to_color32(outline_color);
                let count = text_font::outline_sample_count(style.outline_width);
                for offset in text_font::outline_offsets(count) {
                    painter.galley_with_override_text_color(
                        pos + offset * style.outline_width,
                        galley.clone(),
                        outline_color,
                    );
                }
            }

            if style.bold {
                const BOLD_OFFSET: f32 = 0.6;
                for offset in text_font::outline_offsets(8) {
                    painter.galley_with_override_text_color(
                        pos + offset * BOLD_OFFSET,
                        galley.clone(),
                        color,
                    );
                }
            }

            painter.galley_with_override_text_color(pos, galley, color);
        });
}

/// Rebase pointer positions from the outer window onto the content area, which is egui's own
/// coordinate origin.
///
/// `content_origin` comes from `WindowState::inner_offset()` — i.e. from the very same
/// `Decorations::content_origin()` the content is *drawn* at, rather than being re-derived from
/// the metrics a second time.
///
/// The old form subtracted the border and header in *logical* space and converted back, which
/// agreed with the layout only at integer scale factors: the layout rounds each metric to
/// physical pixels separately, so a fractional factor left the two up to half a pixel apart
/// (+0.5px at 1.5x and 2.5x, -0.25px at 1.25x). Sub-pixel with `plain`'s 1px border — but it
/// also hardcoded that border, so any theme with a thicker one would have been out by whole
/// pixels. Taking the origin from one place makes the two unable to disagree at all.
fn translate_event_position(event: WindowEvent, content_origin: (u32, u32)) -> WindowEvent {
    match event {
        WindowEvent::CursorMoved {
            device_id,
            position,
        } => WindowEvent::CursorMoved {
            device_id,
            position: translate_position(position, content_origin),
        },
        WindowEvent::Touch(Touch {
            device_id,
            phase,
            location,
            force,
            id,
        }) => WindowEvent::Touch(Touch {
            device_id,
            phase,
            location: translate_position(location, content_origin),
            force,
            id,
        }),
        event => event,
    }
}

fn translate_position(
    position: PhysicalPosition<f64>,
    (origin_x, origin_y): (u32, u32),
) -> PhysicalPosition<f64> {
    PhysicalPosition::new(position.x - origin_x as f64, position.y - origin_y as f64)
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalSize;

    use super::*;
    use crate::window::decorations::Decorations;
    use crate::window::redraw::RedrawRequester;
    use crate::window::theme::{Appearance, Metrics};

    /// `plain`'s metrics plus stand-ins for themes yet to be written, so the agreement below is
    /// a property of the seam rather than of one theme's numbers.
    const METRICS: &[Metrics] = &[
        Metrics::PLAIN,
        Metrics {
            header_height: 18,
            border_width: 4,
        },
        Metrics {
            header_height: 37,
            border_width: 1,
        },
    ];

    fn content_origin(metrics: Metrics, scale_factor: f64) -> (u32, u32) {
        Decorations::new(
            true,
            metrics,
            Theme::Plain.chrome(Appearance::Light),
            PhysicalSize::new(200, 150),
            scale_factor,
            None,
            true,
            RedrawRequester::detached(),
        )
        .content_origin()
    }

    /// The regression this seam exists to prevent: egui's coordinate origin has to be the exact
    /// pixel the content is drawn at. A mismatch does not look broken — it silently sends clicks
    /// to the wrong widget — and the old hardcoded translation drifted from `content_origin()`
    /// at every fractional scale factor.
    #[test]
    fn a_pointer_at_the_content_origin_lands_on_eguis_own_origin() {
        for &metrics in METRICS {
            for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
                let origin = content_origin(metrics, scale);
                let at_origin = PhysicalPosition::new(origin.0 as f64, origin.1 as f64);

                assert_eq!(
                    translate_position(at_origin, origin),
                    PhysicalPosition::new(0.0, 0.0),
                    "{metrics:?} at {scale}x"
                );
            }
        }
    }

    /// Offsets inside the content area survive the rebase unchanged, so a click 10px into the
    /// content is a click 10px into egui.
    #[test]
    fn offsets_within_the_content_area_are_preserved() {
        for &metrics in METRICS {
            for scale in [1.0, 1.5, 2.0] {
                let origin = content_origin(metrics, scale);
                let position =
                    PhysicalPosition::new(origin.0 as f64 + 10.0, origin.1 as f64 + 20.0);

                assert_eq!(
                    translate_position(position, origin),
                    PhysicalPosition::new(10.0, 20.0),
                    "{metrics:?} at {scale}x"
                );
            }
        }
    }

    /// A pointer over the decorations themselves rebases to negative coordinates rather than
    /// wrapping into the content area — egui must see it as outside, not as a click near (0, 0).
    #[test]
    fn a_pointer_over_the_header_rebases_outside_the_content() {
        for &metrics in METRICS {
            let origin = content_origin(metrics, 1.0);
            let in_header = PhysicalPosition::new(origin.0 as f64, 0.0);

            let translated = translate_position(in_header, origin);
            assert!(translated.y < 0.0, "{metrics:?} header y={}", translated.y);
        }
    }

    /// Only pointer events are rebased; everything else passes through untouched.
    #[test]
    fn non_pointer_events_pass_through() {
        let event = WindowEvent::Focused(true);
        assert!(matches!(
            translate_event_position(event, (1, 25)),
            WindowEvent::Focused(true)
        ));
    }
}

#[cfg(test)]
mod dialog_layout_tests {
    use super::*;
    use crate::window::theme::Appearance;

    const PANEL: f32 = 400.0;

    fn buttons(count: usize) -> Vec<DialogButton> {
        (0..count)
            .map(|index| DialogButton {
                id: format!("b{index}"),
                label: format!("Button {index}"),
                default: index == 0,
            })
            .collect()
    }

    fn elements(button_count: usize) -> Vec<DialogElementState> {
        vec![
            DialogElementState::Input {
                id: "field".to_owned(),
                placeholder: None,
                value: String::new(),
            },
            DialogElementState::Buttons {
                id: None,
                options: buttons(button_count),
            },
        ]
    }

    /// Lay a dialog out in a `PANEL`-wide viewport and return the rect of every filled rectangle
    /// egui emitted, plus where it put each piece of text.
    ///
    /// `paint_dialog` only needs a `Ui`, so this exercises the real layout with no window, no wgpu
    /// and no egui-winit — which is what makes the geometry testable at all.
    fn layout(button_count: usize) -> (Vec<egui::Rect>, Vec<(String, egui::Pos2)>) {
        layout_themed(Theme::Plain, button_count)
    }

    /// As [`layout`], but with a theme's real style applied — the spacing and control metrics a
    /// theme sets are exactly what can push content out of the region it was allocated, so a
    /// harness on egui's defaults cannot see those problems at all.
    fn layout_themed(
        theme: Theme,
        button_count: usize,
    ) -> (Vec<egui::Rect>, Vec<(String, egui::Pos2)>) {
        let (rects, texts) = layout_shapes(theme, button_count);
        (rects.into_iter().map(|(rect, _)| rect).collect(), texts)
    }

    /// As [`layout_themed`], but keeping each rectangle's fill.
    #[allow(clippy::type_complexity)]
    fn layout_shapes(
        theme: Theme,
        button_count: usize,
    ) -> (Vec<(egui::Rect, egui::Color32)>, Vec<(String, egui::Pos2)>) {
        let ctx = egui::Context::default();
        ctx.set_global_style(theme::window_style(theme, Appearance::Light, None));
        if let Some(fonts) = theme::widget_font_definitions(theme) {
            ctx.set_fonts(fonts);
        }

        let mut elements = elements(button_count);

        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(PANEL, 300.0),
            )),
            ..Default::default()
        };

        let output = ctx.run_ui(input, |ui| {
            paint_dialog(
                ui,
                &mut elements,
                Some("b0"),
                &theme.widget_edge(Appearance::Light),
                theme.default_button_style(Appearance::Light),
            );
        });

        let mut rects = Vec::new();
        let mut texts = Vec::new();
        for shape in &output.shapes {
            match &shape.shape {
                egui::epaint::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
                egui::epaint::Shape::Text(text) => {
                    texts.push((text.galley.text().to_owned(), text.pos))
                }
                _ => {}
            }
        }

        (rects, texts)
    }

    #[test]
    fn unstyled_dialog_text_inherits_the_theme_face_but_explicit_fonts_win() {
        for &theme in crate::window::theme::ALL_THEMES {
            let ctx = egui::Context::default();
            ctx.set_global_style(theme::window_style(theme, Appearance::Light, None));

            let mut inherited = None;
            let mut explicit = None;
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                inherited = Some(dialog_text_family(ui, lua::TextFont::Default));
                explicit = Some(dialog_text_family(ui, lua::TextFont::Mono));
            });

            assert_eq!(
                inherited,
                Some(text_font::font_family(theme.widget_font())),
                "{theme:?}"
            );
            // A named family, not egui's generic `Monospace`: the mono face is bundled like every
            // other, so nothing is left to egui's own choice of file.
            assert_eq!(
                explicit,
                Some(text_font::font_family(lua::TextFont::Mono)),
                "{theme:?}"
            );
        }
    }

    /// egui paints an orange "Unaligned" marker (debug builds only) when a `Ui`'s content overflows
    /// the region it was allocated. That is a layout error, not cosmetic noise, so nothing a dialog
    /// lays out may provoke one.
    #[test]
    fn no_debug_warnings_are_emitted() {
        for &theme in crate::window::theme::ALL_THEMES {
            for count in [1, 2, 8] {
                let (_, texts) = layout_themed(theme, count);
                let warnings: Vec<&String> = texts
                    .iter()
                    .map(|(text, _)| text)
                    .filter(|text| text.contains("Unaligned") || text.contains("Debug"))
                    .collect();

                assert!(
                    warnings.is_empty(),
                    "{theme:?}, {count} buttons: egui flagged the layout: {warnings:?}"
                );
            }
        }
    }

    /// The dialog's background must cover the whole window, not just the strip its content
    /// occupies — otherwise the surface's clear colour (black) shows through beneath.
    ///
    /// This filled by accident until the button row stopped being infinitely wide: the infinite
    /// rect stretched the frame's `min_rect` to the bottom of the window.
    #[test]
    fn the_dialog_background_covers_the_whole_window() {
        for count in [1, 2, 8] {
            let (rects, _) = layout(count);

            let covers = rects.iter().any(|rect| {
                rect.min.x <= 0.5
                    && rect.min.y <= 0.5
                    && rect.max.x >= PANEL - 0.5
                    && rect.max.y >= 299.5
            });

            assert!(
                covers,
                "{count} buttons: nothing fills the 400x300 window; largest was {:?}",
                rects.iter().max_by(|a, b| a.area().total_cmp(&b.area()))
            );
        }
    }

    /// A theme's text field should be the same height as its buttons — a short field beside a tall
    /// button is the sort of mismatch that makes a themed dialog look assembled rather than
    /// designed. egui sizes a `TextEdit` from its font and ignores `interact_size`, so this only
    /// holds because `paint_dialog` pads it out deliberately.
    #[test]
    fn a_text_field_matches_its_themes_control_height() {
        for &theme in crate::window::theme::ALL_THEMES {
            let (rects, _) = layout_themed(theme, 1);

            let input = rects
                .iter()
                .filter(|rect| rect.width() > PANEL / 2.0 && rect.height() < 60.0)
                .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
                .expect("the input should have been drawn");
            let button = button_rects(&rects)
                .into_iter()
                .next()
                .expect("the button should have been drawn");

            let difference = (input.height() - button.height()).abs();
            assert!(
                difference <= 2.0,
                "{theme:?}: the text field is {} tall but its buttons are {}",
                input.height(),
                button.height()
            );
        }
    }

    /// The rects that are buttons: control-sized, and not the panel background.
    ///
    /// The lower bound matters for the bevelled themes: those paint their edges as one-point strips,
    /// so without it a two-button `redmond` dialog reports eighteen "buttons".
    fn button_rects(rects: &[egui::Rect]) -> Vec<egui::Rect> {
        let candidates: Vec<_> = rects
            .iter()
            .copied()
            .filter(|rect| {
                rect.width() < PANEL / 2.0
                    && rect.height() < 40.0
                    && rect.width() > 8.0
                    && rect.height() > 8.0
            })
            .collect();

        // A default-action outline is another control-sized rectangle nested just inside its
        // button. Keep only the outermost candidate so decoration is not mistaken for a second
        // control.
        candidates
            .iter()
            .copied()
            .filter(|rect| {
                !candidates.iter().any(|other| {
                    other != rect
                        && other.contains(rect.min)
                        && other.contains(rect.max)
                        && other.area() > rect.area()
                })
            })
            .collect()
    }

    /// Nothing may be laid out at an infinite coordinate.
    ///
    /// This is what `with_main_justify(true)` did on a *wrapping* layout: a wrapping layout reports
    /// an infinite available main extent, so "fill the main axis" produced a button of infinite
    /// width. Clipped to the panel it looked like a single wide bar, its label was drawn at
    /// x = infinity where nothing is visible, and every button after the first vanished.
    #[test]
    fn nothing_is_laid_out_at_an_infinite_coordinate() {
        for count in [1, 2, 3, 8] {
            let (rects, texts) = layout(count);

            for rect in &rects {
                assert!(
                    rect.min.x.is_finite() && rect.max.x.is_finite(),
                    "{count} buttons: rect {rect:?} is not finite"
                );
            }
            for (text, pos) in &texts {
                assert!(
                    pos.x.is_finite() && pos.y.is_finite(),
                    "{count} buttons: {text:?} placed at {pos:?}"
                );
            }
        }
    }

    /// Every button is drawn, and every label lands inside the button it belongs to.
    #[test]
    fn every_button_is_drawn_with_its_label_inside_it() {
        for &theme in crate::window::theme::ALL_THEMES {
            for count in [1, 2, 3, 8] {
                let (rects, texts) = layout_themed(theme, count);
                let drawn = button_rects(&rects);

                assert_eq!(
                    drawn.len(),
                    count,
                    "{theme:?}, {count} buttons: drew {}",
                    drawn.len()
                );

                for index in 0..count {
                    let label = format!("Button {index}");
                    let (_, pos) = texts
                        .iter()
                        .find(|(text, _)| *text == label)
                        .unwrap_or_else(|| {
                            panic!("{theme:?}, {count} buttons: {label:?} was never drawn")
                        });

                    assert!(
                        drawn.iter().any(|rect| rect.contains(*pos)),
                        "{theme:?}, {count} buttons: {label:?} at {pos:?} is outside every button"
                    );
                }
            }
        }
    }

    /// Flat themes paint the default action as a filled primary button, rather than presenting
    /// keyboard focus as a second outline around an otherwise ordinary control.
    #[test]
    fn flat_themes_fill_only_the_default_button_with_their_primary_colour() {
        for theme in [Theme::Plain, Theme::Fluent, Theme::Aqua, Theme::Adwaita] {
            let (shapes, _) = layout_shapes(theme, 2);
            let rects: Vec<_> = shapes.iter().map(|(rect, _)| *rect).collect();
            let buttons = button_rects(&rects);
            assert_eq!(buttons.len(), 2, "{theme:?}");

            let fills: Vec<_> = buttons
                .iter()
                .map(|button| {
                    shapes
                        .iter()
                        .find(|(rect, _)| rect == button)
                        .expect("button should have a painted face")
                        .1
                })
                .collect();

            assert_ne!(
                fills[0], fills[1],
                "{theme:?}: default face was not distinct"
            );
        }
    }

    /// A bevelled theme's button really is two-toned: its edges are painted in more than one colour,
    /// which is the whole reason those themes are not left to `egui::Style`.
    #[test]
    fn a_bevelled_theme_paints_a_two_tone_edge() {
        for &theme in crate::window::theme::ALL_THEMES {
            let bevelled = matches!(
                theme.widget_edge(Appearance::Light),
                WidgetEdge::Bevel { .. }
            );

            let (shapes, _) = layout_shapes(theme, 1);
            let button = button_rects(&shapes.iter().map(|(rect, _)| *rect).collect::<Vec<_>>())
                .into_iter()
                .next()
                .expect("a button should have been drawn");

            // The one-point strips inside the button's own bounds are its edges.
            let edge_tones: std::collections::BTreeSet<[u8; 4]> = shapes
                .iter()
                .filter(|(rect, _)| {
                    button.contains(rect.center())
                        && (rect.width() <= 2.0 || rect.height() <= 2.0)
                        && rect.width() > 0.0
                        && rect.height() > 0.0
                })
                .map(|(_, fill)| fill.to_array())
                .collect();

            if bevelled {
                assert!(
                    edge_tones.len() >= 2,
                    "{theme:?} claims a bevel but paints {} edge tone(s)",
                    edge_tones.len()
                );
            } else {
                assert!(
                    edge_tones.is_empty(),
                    "{theme:?} is flat but painted edge strips: {edge_tones:?}"
                );
            }
        }
    }

    #[test]
    fn a_bevelled_theme_recesses_its_text_field() {
        for &theme in crate::window::theme::ALL_THEMES {
            let bevelled = matches!(
                theme.widget_edge(Appearance::Light),
                WidgetEdge::Bevel { .. }
            );
            let (shapes, _) = layout_shapes(theme, 1);
            let input = shapes
                .iter()
                .map(|(rect, _)| *rect)
                .filter(|rect| rect.width() > PANEL / 2.0 && rect.height() < 60.0)
                .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
                .expect("the input should have been drawn");
            let edge_tones: std::collections::BTreeSet<_> = shapes
                .iter()
                .filter(|(rect, _)| {
                    input.contains(rect.center())
                        && (rect.width() <= 2.0 || rect.height() <= 2.0)
                        && rect.width() > 0.0
                        && rect.height() > 0.0
                })
                .map(|(_, fill)| fill.to_array())
                .collect();

            if bevelled {
                assert!(edge_tones.len() >= 2, "{theme:?}: field is not recessed");
            } else {
                assert!(
                    edge_tones.is_empty(),
                    "{theme:?}: flat field drew edge strips"
                );
            }
        }
    }

    /// A row that fits is centred, matching the centred text above it. The enclosing
    /// `top_down(Align::Center)` does the centring; the row only has to be allocated at its own
    /// width rather than the full panel's.
    #[test]
    fn a_button_row_that_fits_is_centred_on_one_line() {
        let (rects, _) = layout(2);
        let drawn = button_rects(&rects);
        assert_eq!(drawn.len(), 2);

        // One line.
        assert_eq!(
            drawn[0].min.y, drawn[1].min.y,
            "two buttons should share a row: {drawn:?}"
        );

        let row = drawn[0].union(drawn[1]);
        let offset = (row.center().x - PANEL / 2.0).abs();
        assert!(
            offset < 2.0,
            "row is not centred: centre {}",
            row.center().x
        );
    }

    /// More buttons than fit wrap onto further lines rather than overflowing the dialog, so a mode
    /// that declares a lot of them still leaves every one reachable.
    #[test]
    fn too_many_buttons_wrap_instead_of_overflowing() {
        let (rects, _) = layout(8);
        let drawn = button_rects(&rects);
        assert_eq!(drawn.len(), 8);

        let rows: std::collections::BTreeSet<i32> =
            drawn.iter().map(|rect| rect.min.y as i32).collect();
        assert!(rows.len() > 1, "8 buttons did not wrap: {drawn:?}");

        for rect in &drawn {
            assert!(
                rect.max.x <= PANEL,
                "button {rect:?} overflows the {PANEL}-wide dialog"
            );
        }
    }

    /// The row sits directly beneath the element above it.
    ///
    /// `left_to_right(Align::Center)` sets the *cross* align, and the row's region spans all the
    /// height left in the dialog — so centring stranded the buttons in the middle of the empty
    /// space below the content instead of under it.
    #[test]
    fn the_button_row_follows_the_element_above_it() {
        let (rects, _) = layout(2);
        let drawn = button_rects(&rects);

        // The text field: full-width-ish, but not the panel background.
        let input = rects
            .iter()
            .filter(|rect| rect.width() > PANEL / 2.0 && rect.height() < 40.0)
            .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .expect("the input should have been drawn");

        let gap = drawn[0].min.y - input.max.y;
        assert!(
            (0.0..=24.0).contains(&gap),
            "buttons are {gap} below the input, which is not directly beneath it"
        );
    }
}
