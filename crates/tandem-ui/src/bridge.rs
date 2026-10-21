//! The UI's only way to the core: Tauri's `invoke` and events, and page
//! pixels over the `tandem://` protocol. No other JavaScript is involved.

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    async fn tauri_listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> JsValue;
}

/// Whether the page runs inside the app (false in a plain browser preview).
pub fn in_app() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI__"))
        .map(|v| !v.is_undefined())
        .unwrap_or(false)
}

fn to_js<T: Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

/// Calls a core command; the error is the core's plain-language text.
pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let result = tauri_invoke(cmd, to_js(args))
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Listens to a core event for the life of the page.
pub fn listen<T: DeserializeOwned + 'static>(
    event: &'static str,
    mut handler: impl FnMut(T) + 'static,
) {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |raw: JsValue| {
        let payload =
            js_sys::Reflect::get(&raw, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
        if let Ok(value) = serde_wasm_bindgen::from_value::<T>(payload) {
            handler(value);
        }
    });
    wasm_bindgen_futures::spawn_local(async move {
        let _ = tauri_listen(event, &closure).await;
        // The listener lives as long as the page.
        closure.forget();
    });
}

/// One rendered page: RGBA pixels and their size.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: js_sys::Uint8ClampedArray,
}

/// Fetches page pixels from the core (`tandem://localhost/<kind>/<page>?scale=…&gen=…`).
pub async fn pixels(
    kind: &str,
    page: usize,
    scale: f64,
    generation: u64,
) -> Result<Pixels, String> {
    pixels_hiding(kind, page, scale, generation, None).await
}

/// [`pixels`] with one annotation or widget left out (`&hide=<object number>`).
pub async fn pixels_hiding(
    kind: &str,
    page: usize,
    scale: f64,
    generation: u64,
    hide: Option<i64>,
) -> Result<Pixels, String> {
    let window = web_sys::window().ok_or("no window")?;
    let hide = hide.map(|n| format!("&hide={n}")).unwrap_or_default();
    let url = format!("tandem://localhost/{kind}/{page}?scale={scale:.4}&gen={generation}{hide}");
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "not a response".to_string())?;
    if !response.ok() {
        return Err(format!("page {} could not be drawn", page + 1));
    }
    let headers = response.headers();
    let width: u32 = headers
        .get("x-width")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let height: u32 = headers
        .get("x-height")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let buffer = JsFuture::from(response.array_buffer().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(Pixels {
        width,
        height,
        data: js_sys::Uint8ClampedArray::new(&buffer),
    })
}

/// Paints pixels into a canvas shown at `css_width × css_height`.
pub fn paint(
    canvas: &web_sys::HtmlCanvasElement,
    pixels: &Pixels,
    css_width: f64,
    css_height: f64,
) -> Result<(), String> {
    if pixels.width == 0 || pixels.height == 0 {
        return Err("empty page".into());
    }
    let image = web_sys::ImageData::new_with_js_u8_clamped_array_and_sh(
        &pixels.data,
        pixels.width,
        pixels.height,
    )
    .map_err(|e| format!("{e:?}"))?;
    canvas.set_width(pixels.width);
    canvas.set_height(pixels.height);
    let style = web_sys::HtmlElement::style(canvas);
    let _ = style.set_property("width", &format!("{css_width}px"));
    let _ = style.set_property("height", &format!("{css_height}px"));
    let ctx: web_sys::CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|e| format!("{e:?}"))?
        .ok_or("no 2d context")?
        .dyn_into()
        .map_err(|_| "no 2d context".to_string())?;
    ctx.put_image_data(&image, 0.0, 0.0)
        .map_err(|e| format!("{e:?}"))
}

/// Copies the part of `from` under `rect` (in `from`'s pixels: x, y, width, height) into
/// `to`, which takes that size.
pub fn copy_region(
    from: &web_sys::HtmlCanvasElement,
    to: &web_sys::HtmlCanvasElement,
    rect: [f64; 4],
) -> Result<(), String> {
    let [x, y, w, h] = rect;
    to.set_width(w.max(1.0) as u32);
    to.set_height(h.max(1.0) as u32);
    let ctx: web_sys::CanvasRenderingContext2d = to
        .get_context("2d")
        .map_err(|e| format!("{e:?}"))?
        .ok_or("no 2d context")?
        .dyn_into()
        .map_err(|_| "no 2d context".to_string())?;
    ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
        from, x, y, w, h, 0.0, 0.0, w, h,
    )
    .map_err(|e| format!("{e:?}"))
}

pub fn device_pixel_ratio() -> f64 {
    web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0)
        .max(1.0)
}

pub fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

pub fn prefers_reduced_motion() -> bool {
    web_sys::window()
        .and_then(|w| {
            w.match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .map(|m| m.matches())
        .unwrap_or(false)
}

/// Writes a line to the app's log (development aid).
pub fn log(msg: impl Into<String>) {
    let msg = msg.into();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = invoke::<_, serde_json::Value>("log", &serde_json::json!({ "msg": msg })).await;
    });
}
