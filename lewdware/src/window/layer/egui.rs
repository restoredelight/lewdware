use std::collections::HashMap;

use anyhow::Result;
use egui::TextEdit;
use tokio::sync::mpsc::UnboundedSender;
use winit::{
    dpi::{LogicalPosition, PhysicalPosition},
    event::{Touch, WindowEvent},
};

use crate::lua::{self, DialogButton, DialogElement, DialogElementUpdate, ItemId, TextStyle};
use crate::media::ImageData;
use crate::text_font;
use crate::window::header::HEADER_HEIGHT;
use crate::window::layer::egui_renderer::{EguiCPUWindow, EguiGpuRenderer};
use crate::window::layer::{CpuFrame, GpuFrame, LayerStatus};
use crate::window::state::WindowState;
use crate::window::surface::Buffer;
use crate::window::target::RenderTarget;

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
) -> Option<DialogInteraction> {
    let mut interaction = None;

    egui::Frame::central_panel(ui.style())
        .inner_margin(10.0)
        .show(ui, |ui| {
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
                            let mut input = TextEdit::singleline(value);
                            if let Some(placeholder) = placeholder {
                                input = input.hint_text(placeholder.as_str());
                            }
                            let response = ui.add(input);
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
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center)
                                    .with_main_wrap(true)
                                    .with_main_align(egui::Align::Center)
                                    .with_main_justify(true),
                                |ui| {
                                    for option in options.iter() {
                                        if ui.button(&option.label).clicked() {
                                            interaction =
                                                Some(DialogInteraction::Select(option.id.clone()));
                                        }
                                        ui.add_space(5.0);
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

/// Paint one `text` element inline within the dialog's vertical stack (unlike `paint_text`,
/// which centers within the whole window for a standalone text popup).
fn paint_dialog_text(ui: &mut egui::Ui, text: &str, style: &TextStyle) {
    let font_size = style.font_size.to_pixels(0);
    let font_id = egui::FontId::new(font_size, text_font::font_family(style.font));
    let color = to_color32(style.color);

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

/// Content painted with egui: either a dialog (interactive elements) or a static text popup.
/// The two share all of their plumbing and differ only in what they paint and which Lua
/// operations apply to them.
pub struct EguiLayer {
    paint: EguiPaint,
    backend: EguiBackend,
    /// Kept so the backend can be rebuilt after a render-target fallback.
    background_color: Option<lua::Color>,
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
    fn run(&mut self, ui: &mut egui::Ui) -> Option<DialogInteraction> {
        match self {
            Self::Dialog {
                elements,
                default_button_id,
            } => paint_dialog(ui, elements, default_button_id.as_deref()),
            Self::Text { text, style } => {
                paint_text(ui, text, style);
                None
            }
        }
    }

    /// The font set this content needs. Text popups pick a font per popup; dialogs style each
    /// text element individually and use egui's defaults.
    fn font_definitions(&self) -> Option<egui::FontDefinitions> {
        match self {
            Self::Dialog { .. } => None,
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
    let font_definitions = paint.font_definitions();

    if target.is_gpu() {
        Ok(EguiBackend::Gpu(Box::new(EguiGpuRenderer::new(
            target.wgpu_state(),
            state.window(),
            state.inner_size(),
            state.opacity,
            target.premultiplied_alpha(),
            target.force_opaque(),
            background_color,
            font_definitions,
            state.redraw_requester(),
        )?)))
    } else {
        Ok(EguiBackend::Cpu(Box::new(EguiCPUWindow::new(
            state.window().clone(),
            background_color,
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
                state.window().scale_factor(),
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

        let mut interaction = None;
        gpu.render_to_texture(frame.wgpu, &window, inner_size, |ui| {
            interaction = paint.run(ui);
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
        let mut interaction = None;
        let _ = cpu.redraw(&mut buffer_ref, |ui| {
            interaction = paint.run(ui);
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
                    style.color = color;
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
            let color = to_color32(style.color);

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

fn to_color32(c: lua::Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        (c.a * 255.0).round() as u8,
    )
}

fn translate_event_position(event: WindowEvent, scale_factor: f64) -> WindowEvent {
    match event {
        WindowEvent::CursorMoved {
            device_id,
            position,
        } => WindowEvent::CursorMoved {
            device_id,
            position: translate_position(position, scale_factor),
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
            location: translate_position(location, scale_factor),
            force,
            id,
        }),
        event => event,
    }
}

fn translate_position(position: PhysicalPosition<f64>, scale_factor: f64) -> PhysicalPosition<f64> {
    let mut logical_position: LogicalPosition<f64> = position.to_logical(scale_factor);
    logical_position.x -= 1.0;
    logical_position.y -= 1.0 + HEADER_HEIGHT as f64;

    logical_position.to_physical(scale_factor)
}
