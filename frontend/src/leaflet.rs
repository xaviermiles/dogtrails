use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use js_sys::{Array, Function, Object, Reflect};
use web_sys::HtmlElement;

use crate::Bbox;

pub fn init_map(element: HtmlElement, bbox: Bbox, on_move: impl Fn(Bbox) + 'static) {
    let global = js_sys::global();
    let leaflet = Reflect::get(&global, &JsValue::from_str("L"))
        .expect("Leaflet not loaded");

    let map = call_method(&leaflet, "map", &[element.into()])
        .expect("map init failed");
    let options = Object::new();
    Reflect::set(&options, &JsValue::from_str("maxZoom"), &JsValue::from_f64(18.0)).ok();
    Reflect::set(
        &options,
        &JsValue::from_str("attribution"),
        &JsValue::from_str("© OpenStreetMap contributors"),
    )
    .ok();

    let tile_layer = call_method(
        &leaflet,
        "tileLayer",
        &[
            JsValue::from_str("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png"),
            options.into(),
        ],
    )
    .expect("tile layer init failed");
    call_method(&tile_layer, "addTo", &[map.clone()]).ok();

    let bounds = lat_lng_bounds(&leaflet, bbox);
    call_method(&map, "fitBounds", &[bounds.clone()]).ok();

    let map_handle = map.clone();
    let callback = Closure::wrap(Box::new(move || {
        if let Some(bounds) = get_bounds(&map_handle) {
            on_move(bounds);
        }
    }) as Box<dyn FnMut()>);

    call_method(&map, "on", &[JsValue::from_str("moveend"), callback.as_ref().clone()]).ok();
    call_method(&map, "on", &[JsValue::from_str("zoomend"), callback.as_ref().clone()]).ok();

    callback.forget();
}

fn call_method(target: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let function = Reflect::get(target, &JsValue::from_str(name))?;
    let function = function.dyn_into::<Function>()?;
    function.apply(target, &Array::from_iter(args.iter().cloned()))
}

fn lat_lng_bounds(leaflet: &JsValue, bbox: Bbox) -> JsValue {
    let sw = Array::of2(&JsValue::from_f64(bbox.min_lat), &JsValue::from_f64(bbox.min_lon));
    let ne = Array::of2(&JsValue::from_f64(bbox.max_lat), &JsValue::from_f64(bbox.max_lon));
    call_method(leaflet, "latLngBounds", &[sw.into(), ne.into()])
        .expect("bounds init failed")
}

fn get_bounds(map: &JsValue) -> Option<Bbox> {
    let bounds = call_method(map, "getBounds", &[]).ok()?;
    let sw = call_method(&bounds, "getSouthWest", &[]).ok()?;
    let ne = call_method(&bounds, "getNorthEast", &[]).ok()?;
    let min_lat = Reflect::get(&sw, &JsValue::from_str("lat")).ok()?.as_f64()?;
    let min_lon = Reflect::get(&sw, &JsValue::from_str("lng")).ok()?.as_f64()?;
    let max_lat = Reflect::get(&ne, &JsValue::from_str("lat")).ok()?.as_f64()?;
    let max_lon = Reflect::get(&ne, &JsValue::from_str("lng")).ok()?.as_f64()?;
    Some(Bbox {
        min_lat,
        min_lon,
        max_lat,
        max_lon,
    })
}
