use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use js_sys::{Array, Function, Object, Reflect};
use web_sys::HtmlElement;

use crate::{Bbox, Trail};

use std::rc::Rc;
use std::cell::Cell;

pub struct MapHandle {
    #[allow(dead_code)]
    map: JsValue,
    leaflet: JsValue,
    marker_layer: JsValue,
}

pub fn init_map(element: HtmlElement, bbox: Bbox, on_move: impl Fn(Bbox) + 'static) -> MapHandle {
    let global = js_sys::global();
    let leaflet = Reflect::get(&global, &JsValue::from_str("L"))
        .expect("Leaflet not loaded");
    let on_move = Rc::new(on_move);

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

    let marker_layer = call_method(&leaflet, "layerGroup", &[])
        .expect("layerGroup init failed");
    call_method(&marker_layer, "addTo", &[map.clone()]).ok();

    let map_for_callback = map.clone();
    let pending_timer = Rc::new(Cell::new(0i32));
    let timer_ref = pending_timer.clone();
    let callback = Closure::wrap(Box::new(move || {
        let old = timer_ref.get();
        if old != 0 {
            let window = web_sys::window().unwrap();
            window.clear_timeout_with_handle(old);
        }
        let map_clone = map_for_callback.clone();
        let on_move_ref = on_move.clone();
        let inner = Closure::once_into_js(move || {
            if let Some(bounds) = get_bounds(&map_clone) {
                on_move_ref(bounds);
            }
        });
        let window = web_sys::window().unwrap();
        let handle = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                inner.unchecked_ref(),
                500,
            )
            .unwrap_or(0);
        timer_ref.set(handle);
    }) as Box<dyn FnMut()>);

    call_method(&map, "on", &[JsValue::from_str("moveend"), callback.as_ref().clone()]).ok();

    callback.forget();

    MapHandle { map, leaflet, marker_layer }
}

pub fn update_markers(handle: &MapHandle, trails: &[Trail]) {
    call_method(&handle.marker_layer, "clearLayers", &[]).ok();
    for trail in trails {
        if trail.lat == 0.0 && trail.lon == 0.0 {
            continue;
        }
        let latlng = Array::of2(
            &JsValue::from_f64(trail.lat),
            &JsValue::from_f64(trail.lon),
        );
        let marker = call_method(&handle.leaflet, "marker", &[latlng.into()])
            .expect("marker failed");
        call_method(&marker, "bindPopup", &[JsValue::from_str(&trail.name)]).ok();
        call_method(&marker, "addTo", &[handle.marker_layer.clone()]).ok();
    }
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
