use std::cell::RefCell;
use std::fmt::Display;
use std::path::PathBuf;
use std::rc::Rc;
use vrm_adapter_wgpu::{WgpuCanvasViewer, WgpuVrmViewerOptions, animation_from_loaded};
use vrm_io::load_vrm_from_slice;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

thread_local! {
    static VIEWERS: RefCell<Vec<ViewerLoop>> = const { RefCell::new(Vec::new()) };
}

struct ViewerLoop {
    _viewer: Rc<RefCell<WgpuCanvasViewer>>,
    _callback: SharedFrameCallback,
}

type SharedFrameCallback = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

#[wasm_bindgen]
pub async fn start_wgpu_vrm_viewer(
    canvas_id: String,
    avatar_bytes: Vec<u8>,
    animation_bytes: Vec<u8>,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let canvas = canvas_by_id(&canvas_id)?;
    let avatar = load_vrm_from_slice(&avatar_bytes).map_err(js_error)?;
    let animation = if animation_bytes.is_empty() {
        None
    } else {
        let loaded = load_vrm_from_slice(&animation_bytes).map_err(js_error)?;
        animation_from_loaded(&loaded)
    };

    let options = WgpuVrmViewerOptions {
        avatar: PathBuf::new(),
        animation: PathBuf::new(),
        no_animation: animation.is_none(),
        speed: 1.0,
        camera_z: 3.0,
        look_y: 1.1,
        width: canvas.width(),
        height: canvas.height(),
    };
    let viewer = Rc::new(RefCell::new(
        WgpuCanvasViewer::new(canvas, &options, avatar, animation)
            .await
            .map_err(js_error)?,
    ));
    let viewer_loop = start_render_loop(Rc::clone(&viewer))?;
    VIEWERS.with(|viewers| viewers.borrow_mut().push(viewer_loop));
    Ok(())
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

fn start_render_loop(viewer: Rc<RefCell<WgpuCanvasViewer>>) -> Result<ViewerLoop, JsValue> {
    let callback: SharedFrameCallback = Rc::new(RefCell::new(None));
    let callback_for_frame = Rc::clone(&callback);
    let viewer_for_frame = Rc::clone(&viewer);

    *callback.borrow_mut() = Some(Closure::wrap(Box::new(move |_time: f64| {
        if let Err(error) = viewer_for_frame.borrow_mut().render_frame() {
            web_sys::console::error_1(&JsValue::from_str(&format!("render failed: {error}")));
            return;
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
        _viewer: viewer,
        _callback: callback,
    })
}

fn request_animation_frame(callback: &Closure<dyn FnMut(f64)>) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window is not available"))?;
    window
        .request_animation_frame(callback.as_ref().unchecked_ref())
        .map(|_| ())
}

fn js_error(error: impl Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
