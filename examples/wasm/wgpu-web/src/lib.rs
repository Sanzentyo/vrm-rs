use egui_wgpu::wgpu;
use std::cell::RefCell;
use std::error::Error;
use std::fmt::Display;
use std::path::PathBuf;
use std::rc::Rc;
use vrm_adapter_wgpu::{
    WgpuCanvasViewer, WgpuOverlayFrame, WgpuVrmViewerOptions, animation_from_loaded,
};
use vrm_core::VrmAnimation;
use vrm_io::load_vrm_from_slice;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

thread_local! {
    static VIEWERS: RefCell<Vec<ViewerLoop>> = const { RefCell::new(Vec::new()) };
}

const MOTION_TRANSITION_SECONDS: f32 = 0.35;

struct ViewerLoop {
    viewer: Rc<RefCell<WgpuCanvasViewer>>,
    ui: Rc<RefCell<WasmEguiOverlay>>,
    _animations: Rc<Vec<Option<VrmAnimation>>>,
    _callback: SharedFrameCallback,
}

struct WasmEguiOverlay {
    context: egui::Context,
    renderer: Option<egui_wgpu::Renderer>,
    events: Vec<egui::Event>,
    selected_animation: usize,
    pending_animation: Option<usize>,
}

type SharedFrameCallback = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

#[wasm_bindgen]
pub async fn start_wgpu_vrm_viewer(
    canvas_id: String,
    avatar_bytes: Vec<u8>,
    animation_a_bytes: Vec<u8>,
    animation_b_bytes: Vec<u8>,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let canvas = canvas_by_id(&canvas_id)?;
    let avatar = load_vrm_from_slice(&avatar_bytes).map_err(js_error)?;
    let animations = Rc::new(vec![
        animation_from_bytes(&animation_a_bytes)?,
        animation_from_bytes(&animation_b_bytes)?,
    ]);
    let initial_animation = animations.first().cloned().flatten();

    let options = WgpuVrmViewerOptions {
        avatar: PathBuf::new(),
        animation: PathBuf::new(),
        no_animation: initial_animation.is_none(),
        speed: 1.0,
        camera_z: 3.0,
        look_y: 1.1,
        width: canvas.width(),
        height: canvas.height(),
    };
    let viewer = Rc::new(RefCell::new(
        WgpuCanvasViewer::new(canvas, &options, avatar, initial_animation)
            .await
            .map_err(js_error)?,
    ));
    let ui = Rc::new(RefCell::new(WasmEguiOverlay::new()));
    let viewer_loop = start_render_loop(Rc::clone(&viewer), Rc::clone(&ui), animations)?;
    VIEWERS.with(|viewers| viewers.borrow_mut().push(viewer_loop));
    Ok(())
}

#[wasm_bindgen]
pub fn pointer_move_wgpu_vrm_ui(x: f32, y: f32) -> Result<(), JsValue> {
    with_latest_ui(|ui| ui.pointer_move(x, y))
}

#[wasm_bindgen]
pub fn pointer_button_wgpu_vrm_ui(x: f32, y: f32, pressed: bool) -> Result<(), JsValue> {
    with_latest_ui(|ui| ui.pointer_button(x, y, pressed))
}

#[wasm_bindgen]
pub fn pointer_leave_wgpu_vrm_ui() -> Result<(), JsValue> {
    with_latest_ui(WasmEguiOverlay::pointer_gone)
}

#[wasm_bindgen]
pub fn orbit_wgpu_vrm_camera(delta_x: f32, delta_y: f32) -> Result<(), JsValue> {
    with_latest_viewer(|viewer| viewer.orbit_camera(delta_x, delta_y))
}

#[wasm_bindgen]
pub fn pan_wgpu_vrm_camera(delta_x: f32, delta_y: f32) -> Result<(), JsValue> {
    with_latest_viewer(|viewer| viewer.pan_camera(delta_x, delta_y))
}

#[wasm_bindgen]
pub fn zoom_wgpu_vrm_camera(scroll_lines: f32) -> Result<(), JsValue> {
    with_latest_viewer(|viewer| viewer.zoom_camera(scroll_lines))
}

#[wasm_bindgen]
pub fn reset_wgpu_vrm_camera() -> Result<(), JsValue> {
    with_latest_viewer(WgpuCanvasViewer::reset_camera)
}

impl WasmEguiOverlay {
    fn new() -> Self {
        let context = egui::Context::default();
        apply_modern_visuals(&context);
        Self {
            context,
            renderer: None,
            events: Vec::new(),
            selected_animation: 0,
            pending_animation: None,
        }
    }

    fn pointer_move(&mut self, x: f32, y: f32) {
        self.events
            .push(egui::Event::PointerMoved(egui::pos2(x, y)));
    }

    fn pointer_button(&mut self, x: f32, y: f32, pressed: bool) {
        self.events.push(egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }

    fn pointer_gone(&mut self) {
        self.events.push(egui::Event::PointerGone);
    }

    fn take_pending_animation(&mut self) -> Option<usize> {
        self.pending_animation.take()
    }

    fn confirm_animation(&mut self, animation: usize) {
        self.selected_animation = animation;
    }

    fn render(&mut self, frame: WgpuOverlayFrame<'_>) -> Result<(), Box<dyn Error>> {
        let pixels_per_point = window_pixels_per_point();
        let screen_size = egui::vec2(
            frame.width as f32 / pixels_per_point,
            frame.height as f32 / pixels_per_point,
        );
        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_size)),
            time: Some(frame_time_seconds()),
            predicted_dt: 1.0 / 60.0,
            events: std::mem::take(&mut self.events),
            focused: true,
            ..Default::default()
        };
        if let Some(viewport) = raw_input.viewports.get_mut(&raw_input.viewport_id) {
            viewport.native_pixels_per_point = Some(pixels_per_point);
        }

        let current_animation = self.selected_animation;
        self.context.begin_pass(raw_input);
        let requested_animation = show_animation_switcher(&self.context, current_animation);
        let full_output = self.context.end_pass();
        if let Some(animation) = requested_animation {
            self.pending_animation = Some(animation);
        }

        let egui::FullOutput {
            textures_delta,
            shapes,
            pixels_per_point,
            ..
        } = full_output;
        let paint_jobs = self.context.tessellate(shapes, pixels_per_point);
        let renderer = self.renderer.get_or_insert_with(|| {
            egui_wgpu::Renderer::new(
                frame.device,
                frame.format,
                egui_wgpu::RendererOptions::default(),
            )
        });
        for (id, image_delta) in &textures_delta.set {
            renderer.update_texture(frame.device, frame.queue, *id, image_delta);
        }

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [frame.width, frame.height],
            pixels_per_point,
        };
        let user_command_buffers = renderer.update_buffers(
            frame.device,
            frame.queue,
            frame.encoder,
            &paint_jobs,
            &screen_descriptor,
        );
        if !user_command_buffers.is_empty() {
            frame.queue.submit(user_command_buffers);
        }

        {
            let render_pass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vrm-rs wasm egui overlay"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: frame.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            renderer.render(
                &mut render_pass.forget_lifetime(),
                &paint_jobs,
                &screen_descriptor,
            );
        }

        for id in &textures_delta.free {
            renderer.free_texture(id);
        }
        Ok(())
    }
}

fn show_animation_switcher(context: &egui::Context, selected_animation: usize) -> Option<usize> {
    let mut requested_animation = None;
    egui::Area::new(egui::Id::new("animation_switcher"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(18.0, 18.0))
        .order(egui::Order::Foreground)
        .show(context, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(18, 22, 30, 188))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(210, 230, 255, 62),
                ))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_min_width(360.0);
                    ui.label(
                        egui::RichText::new("Animation")
                            .size(12.0)
                            .color(egui::Color32::from_rgba_unmultiplied(226, 235, 246, 210)),
                    );
                    ui.add_space(10.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 46.0),
                        egui::Layout::left_to_right(egui::Align::Center)
                            .with_main_align(egui::Align::Center),
                        |ui| {
                            if animation_button(ui, "Motion A", selected_animation == 0).clicked() {
                                requested_animation = Some(0);
                            }
                            if animation_button(ui, "Motion B", selected_animation == 1).clicked() {
                                requested_animation = Some(1);
                            }
                        },
                    );
                    ui.label(
                        egui::RichText::new("0.35 s linear crossfade")
                            .size(11.0)
                            .color(egui::Color32::from_rgba_unmultiplied(226, 235, 246, 150)),
                    );
                });
        });
    requested_animation
}

fn animation_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let fill = if selected {
        egui::Color32::from_rgba_unmultiplied(79, 145, 255, 216)
    } else {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 32)
    };
    let stroke = if selected {
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(230, 242, 255, 120),
        )
    } else {
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(210, 225, 246, 56),
        )
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(16.0)
                .strong()
                .color(egui::Color32::from_rgb(245, 248, 252)),
        )
        .min_size(egui::vec2(154.0, 42.0))
        .fill(fill)
        .stroke(stroke)
        .corner_radius(8),
    )
}

fn apply_modern_visuals(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::TRANSPARENT;
    visuals.window_fill = egui::Color32::from_rgba_unmultiplied(18, 22, 30, 188);
    visuals.window_stroke = egui::Stroke::new(
        1.0,
        egui::Color32::from_rgba_unmultiplied(210, 230, 255, 62),
    );
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 28);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 56);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(79, 145, 255, 216);
    visuals.widgets.inactive.corner_radius = 8.into();
    visuals.widgets.hovered.corner_radius = 8.into();
    visuals.widgets.active.corner_radius = 8.into();
    context.set_visuals(visuals);

    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(16.0, 10.0);
    context.set_style_of(egui::Theme::Dark, style);
}

fn canvas_by_id(id: &str) -> Result<web_sys::HtmlCanvasElement, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window is not available"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("document is not available"))?;
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str("canvas element was not found"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("element is not a canvas"))
}

fn animation_from_bytes(bytes: &[u8]) -> Result<Option<VrmAnimation>, JsValue> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let loaded = load_vrm_from_slice(bytes).map_err(js_error)?;
    Ok(animation_from_loaded(&loaded))
}

fn start_render_loop(
    viewer: Rc<RefCell<WgpuCanvasViewer>>,
    ui: Rc<RefCell<WasmEguiOverlay>>,
    animations: Rc<Vec<Option<VrmAnimation>>>,
) -> Result<ViewerLoop, JsValue> {
    let callback: SharedFrameCallback = Rc::new(RefCell::new(None));
    let callback_for_frame = Rc::clone(&callback);
    let viewer_for_frame = Rc::clone(&viewer);
    let ui_for_frame = Rc::clone(&ui);
    let animations_for_frame = Rc::clone(&animations);

    *callback.borrow_mut() = Some(Closure::wrap(Box::new(move |_time: f64| {
        let render_result = {
            let mut viewer = viewer_for_frame.borrow_mut();
            let mut ui = ui_for_frame.borrow_mut();
            viewer.render_frame_with_overlay(|frame| ui.render(frame))
        };
        if let Err(error) = render_result {
            web_sys::console::error_1(&JsValue::from_str(&format!("render failed: {error}")));
            return;
        }

        let pending_animation = ui_for_frame.borrow_mut().take_pending_animation();
        if let Some(index) = pending_animation {
            let animation = animations_for_frame.get(index).cloned().flatten();
            let animation_result = viewer_for_frame
                .borrow_mut()
                .transition_animation(animation, MOTION_TRANSITION_SECONDS);
            match animation_result {
                Ok(()) => ui_for_frame.borrow_mut().confirm_animation(index),
                Err(error) => {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "animation switch failed: {error}"
                    )));
                }
            }
        }

        if let Some(callback) = callback_for_frame.borrow().as_ref()
            && let Err(error) = request_animation_frame(callback)
        {
            web_sys::console::error_1(&error);
        }
    }) as Box<dyn FnMut(f64)>));

    if let Some(callback) = callback.borrow().as_ref() {
        request_animation_frame(callback)?;
    }
    Ok(ViewerLoop {
        viewer,
        ui,
        _animations: animations,
        _callback: callback,
    })
}

fn with_latest_viewer(action: impl FnOnce(&mut WgpuCanvasViewer)) -> Result<(), JsValue> {
    VIEWERS.with(|viewers| {
        let mut viewers = viewers.borrow_mut();
        let viewer_loop = viewers
            .last_mut()
            .ok_or_else(|| JsValue::from_str("viewer has not started"))?;
        let mut viewer = viewer_loop.viewer.borrow_mut();
        action(&mut viewer);
        Ok(())
    })
}

fn with_latest_ui(action: impl FnOnce(&mut WasmEguiOverlay)) -> Result<(), JsValue> {
    VIEWERS.with(|viewers| {
        let mut viewers = viewers.borrow_mut();
        let viewer_loop = viewers
            .last_mut()
            .ok_or_else(|| JsValue::from_str("viewer has not started"))?;
        let mut ui = viewer_loop.ui.borrow_mut();
        action(&mut ui);
        Ok(())
    })
}

fn request_animation_frame(callback: &Closure<dyn FnMut(f64)>) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window is not available"))?;
    window
        .request_animation_frame(callback.as_ref().unchecked_ref())
        .map(|_| ())
}

fn frame_time_seconds() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now() / 1000.0)
        .unwrap_or(0.0)
}

fn window_pixels_per_point() -> f32 {
    web_sys::window()
        .map(|window| window.device_pixel_ratio() as f32)
        .unwrap_or(1.0)
        .max(1.0)
}

fn js_error(error: impl Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
