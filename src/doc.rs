/// Department of Conservation tracks API integration.
use serde_json::Value;

use crate::{Bbox, Difficulty, DogPolicy, Provider, Trail, TrailError};

const DOC_TRACKS_URL: &str = "https://api.doc.govt.nz/v1/tracks?coordinates=wgs84";

/// Fetch all tracks from the DOC list endpoint (no detail calls).
/// Returns lightweight Trail objects built from summary data only.
pub(crate) async fn fetch_doc_summaries(
    client: &reqwest::Client,
    api_key: &str,
) -> Result<Vec<Trail>, TrailError> {
    let response = client
        .get(DOC_TRACKS_URL)
        .header("x-api-key", api_key)
        .send()
        .await
        .map_err(|err| TrailError(format!("DOC tracks request failed: {err}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string());
        return Err(TrailError(format!(
            "DOC tracks request failed with status {}: {}",
            status, body
        )));
    }

    let payload: Value = response
        .json()
        .await
        .map_err(|err| TrailError(format!("DOC tracks response parse failed: {err}")))?;

    let items = extract_doc_items(&payload);
    tracing::info!("DOC API returned {} tracks total", items.len());

    let trails: Vec<Trail> = items
        .iter()
        .filter_map(|item| map_doc_summary(item))
        .collect();

    tracing::info!("DOC: {} trails after mapping summaries", trails.len());
    Ok(trails)
}

/// Fetch the detail JSON for a single track.
pub(crate) async fn fetch_doc_detail(
    client: &reqwest::Client,
    api_key: &str,
    track_id: &str,
) -> Result<Value, TrailError> {
    let url = format!("https://api.doc.govt.nz/v1/tracks/{}/detail?coordinates=wgs84", track_id);
    let response = client
        .get(url)
        .header("x-api-key", api_key)
        .send()
        .await
        .map_err(|err| TrailError(format!("DOC detail request failed: {err}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string());
        return Err(TrailError(format!(
            "DOC detail request failed with status {}: {}",
            status, body
        )));
    }

    response
        .json::<Value>()
        .await
        .map_err(|err| TrailError(format!("DOC detail response parse failed: {err}")))
}

fn extract_doc_items(payload: &Value) -> Vec<Value> {
    match payload {
        Value::Array(items) => items.clone(),
        Value::Object(map) => map
            .get("tracks")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn extract_doc_id(item: &Value) -> Option<String> {
    item.get("assetId")?.as_str().map(|s| s.to_string())
}

fn map_doc_summary(summary: &Value) -> Option<Trail> {
    let name = doc_string(summary, &["name", "trackName", "title"])?;

    let (dog_policy, dog_notes) = doc_dog_policy_single(summary);

    let location = doc_string(
        summary,
        &["locationString", "locationArray", "location", "region", "district", "place", "area"],
    )
    .unwrap_or_else(|| "New Zealand".to_string());

    let surface = doc_string(summary, &["surface", "trackSurface", "terrain"])
        .unwrap_or_else(|| "Unknown".to_string());

    let distance_km = doc_distance_km_single(summary).unwrap_or(0.0);

    let difficulty = doc_difficulty_single(summary)
        .unwrap_or_else(|| crate::map_difficulty(None, distance_km));

    let map_url = doc_string(summary, &["staticLink", "url", "webUrl", "docUrl", "link"])
        .unwrap_or_else(|| "https://www.doc.govt.nz".to_string());

    let id = extract_doc_id(summary)
        .unwrap_or_else(|| name.to_lowercase().replace(' ', "-"));

    let (trail_lat, trail_lon) = extract_lat_lon(summary).unwrap_or((0.0, 0.0));

    let line = extract_line_coords(summary).unwrap_or_default();
    let line_bbox = extract_line_bbox(summary).unwrap_or(Bbox {
        min_lat: trail_lat,
        min_lon: trail_lon,
        max_lat: trail_lat,
        max_lon: trail_lon,
    });

    Some(Trail {
        id,
        name,
        provider: Provider::DOC,
        location,
        distance_km,
        elevation_m: None,
        difficulty,
        dog_policy,
        dog_notes,
        surface,
        map_url,
        lat: trail_lat,
        lon: trail_lon,
        line,
        line_bbox,
    })
}

/// Enrich a trail with fields from the detail endpoint, filling in
/// any data the summary was missing.
pub(crate) fn enrich_with_detail(trail: &mut Trail, detail: &Value) {
    // Prefer detail values for fields that are often richer
    if let Some(name) = doc_string(detail, &["name", "trackName", "title"]) {
        trail.name = name;
    }
    if let Some(loc) = doc_string(
        detail,
        &["locationString", "locationArray", "location", "region", "district", "place", "area"],
    ) {
        trail.location = loc;
    }
    if let Some(km) = doc_distance_km_single(detail) {
        if trail.distance_km == 0.0 || km > 0.0 {
            trail.distance_km = km;
        }
    }
    if let Some(diff) = doc_difficulty_single(detail) {
        trail.difficulty = diff;
    }
    let (dog_policy, dog_notes) = doc_dog_policy_single(detail);
    if dog_policy != DogPolicy::Unknown {
        trail.dog_policy = dog_policy;
        trail.dog_notes = dog_notes;
    }
    if let Some(surface) = doc_string(detail, &["surface", "trackSurface", "terrain"]) {
        trail.surface = surface;
    }
    if let Some(url) = doc_string(detail, &["staticLink", "url", "webUrl", "docUrl", "link"]) {
        trail.map_url = url;
    }
    // Fill in line coords from detail if summary had none
    if trail.line.is_empty() {
        if let Some(line) = extract_line_coords(detail) {
            trail.line = line;
        }
    }
    if let Some(lb) = extract_line_bbox(detail) {
        trail.line_bbox = lb;
    }
    if let Some((lat, lon)) = extract_lat_lon(detail) {
        if trail.lat == 0.0 && trail.lon == 0.0 {
            trail.lat = lat;
            trail.lon = lon;
        }
    }
}

fn doc_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(field) = value.get(*key) {
            if let Some(text) = field.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            // Handle arrays of strings (e.g. DOC "region": ["Canterbury"])
            if let Some(arr) = field.as_array() {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect();
                if !parts.is_empty() {
                    return Some(parts.join(", "));
                }
            }
        }
    }
    None
}

fn doc_number(value: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(field) = value.get(*key) {
            if let Some(num) = field.as_f64() {
                return Some(num);
            }
            if let Some(text) = field.as_str() {
                if let Some(parsed) = parse_number(text) {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

fn parse_number(text: &str) -> Option<f64> {
    let mut buf = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            buf.push(ch);
        } else if !buf.is_empty() {
            break;
        }
    }
    if buf.is_empty() {
        None
    } else {
        buf.parse::<f64>().ok()
    }
}

fn doc_distance_km_single(value: &Value) -> Option<f32> {
    let raw = doc_number(value, &["distance", "distanceKm", "length", "trackLength"])?;
    let text = doc_string(value, &["distance", "distanceKm", "length", "trackLength"]);
    if let Some(text) = text {
        let lower = text.to_lowercase();
        if lower.contains(" m") && !lower.contains("km") {
            return Some((raw / 1000.0) as f32);
        }
    }
    if raw > 1000.0 {
        Some((raw / 1000.0) as f32)
    } else {
        Some(raw as f32)
    }
}

fn doc_difficulty_single(value: &Value) -> Option<Difficulty> {
    let text = doc_string(value, &["difficulty", "grade", "trackGrade", "walkTrackCategory"])?;
    let lower = text.to_lowercase();
    if lower.contains("easy") {
        Some(Difficulty::Easy)
    } else if lower.contains("moderate") || lower.contains("intermediate") {
        Some(Difficulty::Moderate)
    } else if lower.contains("hard") || lower.contains("advanced") || lower.contains("expert") {
        Some(Difficulty::Hard)
    } else {
        None
    }
}

fn doc_dog_policy_single(value: &Value) -> (DogPolicy, Option<String>) {
    let allowed = doc_bool(value, &["dogsAllowed", "dogAllowed"]);
    let on_lead = doc_bool(value, &["dogsAllowedOnLead", "dogsOnLead"]);
    if let Some(false) = allowed {
        return (DogPolicy::NotAllowed, Some("Dogs are not permitted.".to_string()));
    }
    if let Some(true) = allowed {
        if let Some(true) = on_lead {
            return (DogPolicy::Partial, Some("Dogs must be on a lead.".to_string()));
        }
        return (DogPolicy::Allowed, None);
    }

    if let Some(text) = doc_string(value, &["dogsAllowed"]) {
        if text.contains("Dogs with a DOC permit for recreational hunting or management purposes only.") {
            return (DogPolicy::HuntingPermit, Option::None);
        }
        if text.contains("Dogs on a leash only. Other pets on conservation land rules.") {
            // TODO: add DogPolicy::LeashOnly and remove Partial
            return (DogPolicy::Partial, Option::None);
        }
        if text.contains("No dogs. Other pets on conservation land rules.") {
            return (DogPolicy::NotAllowed, Option::None);
        }
        return (DogPolicy::Unknown, Some(text));
    }

    (
        DogPolicy::Unknown,
        Some("???".to_string()),
    )
}

fn doc_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(field) = value.get(*key) {
            if let Some(flag) = field.as_bool() {
                return Some(flag);
            }
            if let Some(text) = field.as_str() {
                let lower = text.to_lowercase();
                if lower == "yes" || lower == "true" {
                    return Some(true);
                }
                if lower == "no" || lower == "false" {
                    return Some(false);
                }
            }
        }
    }
    None
}

fn extract_lat_lon(value: &Value) -> Option<(f64, f64)> {
    // Try explicit lat/lon keys
    if let (Some(lat), Some(lon)) = (
        doc_number(value, &["latitude", "lat", "y"]),
        doc_number(value, &["longitude", "lon", "lng", "x"]),
    ) {
        return Some((lat, lon));
    }

    if let Some(coords) = value.get("coordinates").and_then(|v| v.as_array()) {
        if coords.len() >= 2 {
            if let (Some(lon), Some(lat)) = (coords[0].as_f64(), coords[1].as_f64()) {
                return Some((lat, lon));
            }
        }
    }

    for key in ["location", "centroid", "position"] {
        if let Some(child) = value.get(key) {
            if let Some(found) = extract_lat_lon(child) {
                return Some(found);
            }
        }
    }

    None
}

fn bbox_intersects(a: Bbox, b: Bbox) -> bool {
    a.min_lat <= b.max_lat
        && a.max_lat >= b.min_lat
        && a.min_lon <= b.max_lon
        && a.max_lon >= b.min_lon
}

/// Compute a bounding box from the DOC `line` field (array of [lon, lat] pairs).
fn extract_line_bbox(value: &Value) -> Option<Bbox> {
    let line = value.get("line")?.as_array()?;
    let mut min_lat = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut min_lon = f64::MAX;
    let mut max_lon = f64::MIN;
    let mut found = false;

    for segment in line {
        let points = match segment.as_array() {
            Some(pts) => pts.as_slice(),
            None => continue,
        };
        for point in points {
            if let Some(pair) = point.as_array() {
                // [lon, lat] GeoJSON order
                if pair.len() >= 2 {
                    if let (Some(lon), Some(lat)) = (pair[0].as_f64(), pair[1].as_f64()) {
                        min_lat = min_lat.min(lat);
                        max_lat = max_lat.max(lat);
                        min_lon = min_lon.min(lon);
                        max_lon = max_lon.max(lon);
                        found = true;
                    }
                }
            }
        }
    }

    if found {
        Some(Bbox {
            min_lat,
            min_lon,
            max_lat,
            max_lon,
        })
    } else {
        None
    }
}

/// Filter DOC trails: include if the track's line bbox intersects the view.
pub(crate) fn filter_doc_by_bbox(trails: &[Trail], view: Bbox) -> Vec<Trail> {
    let filtered_trails = trails
        .iter()
        .filter(|trail| bbox_intersects(view, trail.line_bbox))
        .cloned()
        .collect::<Vec<_>>();
    tracing::info!("DOC filtered by bounding box gives {} tracks total", filtered_trails.len());
    filtered_trails
}

/// Extract line coordinates as `[[lat, lon], ...]` from the DOC `line` field.
fn extract_line_coords(value: &Value) -> Option<Vec<[f64; 2]>> {
    let line = value.get("line")?.as_array()?;
    let mut coords = Vec::new();

    for segment in line {
        let points = match segment.as_array() {
            Some(pts) => pts.as_slice(),
            None => continue,
        };
        for point in points {
            if let Some(pair) = point.as_array() {
                // [lon, lat] GeoJSON order → [lat, lon] for Leaflet
                if pair.len() >= 2 {
                    if let (Some(lon), Some(lat)) = (pair[0].as_f64(), pair[1].as_f64()) {
                        coords.push([lat, lon]);
                    }
                }
            }
        }
    }

    if coords.is_empty() { None } else { Some(coords) }
}
