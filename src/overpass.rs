/// Overpass API integration.
use std::time::Duration;

use serde::Deserialize;

use crate::{Bbox, DogPolicy, Provider, Trail, TrailError};

#[derive(Deserialize)]
struct OverpassResponse {
    elements: Vec<OverpassElement>,
}

#[derive(Deserialize)]
struct OverpassElement {
    #[serde(rename = "type")]
    element_type: String,
    id: u64,
    tags: Option<std::collections::HashMap<String, String>>,
    geometry: Option<Vec<OverpassPoint>>,
    center: Option<OverpassPoint>,
}

#[derive(Deserialize)]
struct OverpassPoint {
    lat: f64,
    lon: f64,
}

pub(crate) async fn fetch_overpass_with_fallback(
    client: &reqwest::Client,
    overpass_urls: &[String],
    bbox: Bbox,
) -> Result<Vec<Trail>, TrailError> {
    let mut last_error: Option<TrailError> = None;
    for url in overpass_urls {
        match fetch_overpass_trails(client, url, bbox).await {
            Ok(trails) => return Ok(trails),
            Err(err) => {
                tracing::warn!("overpass request failed for {}: {}", url, err);
                last_error = Some(err);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| TrailError("no overpass endpoints configured".to_string())))
}

async fn fetch_overpass_trails(
    client: &reqwest::Client,
    overpass_url: &str,
    bbox: Bbox,
) -> Result<Vec<Trail>, TrailError> {
    let query = format!(
        "[out:json][timeout:25];(way[highway=path][dog]({min_lat},{min_lon},{max_lat},{max_lon});way[highway=footway][dog]({min_lat},{min_lon},{max_lat},{max_lon});way[route=hiking][dog]({min_lat},{min_lon},{max_lat},{max_lon}););out tags center;",
        min_lat = bbox.min_lat,
        min_lon = bbox.min_lon,
        max_lat = bbox.max_lat,
        max_lon = bbox.max_lon
    );

    if query.trim().is_empty() {
        return Err(TrailError("overpass query is empty".to_string()));
    }

    let url = append_overpass_query(overpass_url, &query);

    let max_retries = 3;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|err| TrailError(format!("overpass request failed: {err}")))?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            if attempt >= max_retries {
                return Err(TrailError("overpass rate limited after retries".to_string()));
            }
            let delay = Duration::from_secs(2u64.pow(attempt as u32));
            tracing::warn!(
                "overpass 429, retrying in {:?} (attempt {}/{})",
                delay,
                attempt,
                max_retries
            );
            tokio::time::sleep(delay).await;
            continue;
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(TrailError(format!(
                "overpass request failed with status {}: {}",
                status, body
            )));
        }

        let data: OverpassResponse = response
            .json()
            .await
            .map_err(|err| TrailError(format!("overpass response parse failed: {err}")))?;

        return Ok(data
            .elements
            .into_iter()
            .filter(|element| element.element_type == "way")
            .filter_map(map_overpass_element)
            .collect());
    }
}

fn append_overpass_query(base_url: &str, query: &str) -> String {
    let encoded = urlencoding::encode(query);
    if base_url.contains('?') {
        format!("{}&data={}", base_url, encoded)
    } else {
        format!("{}?data={}", base_url, encoded)
    }
}

fn map_overpass_element(element: OverpassElement) -> Option<Trail> {
    let tags = element.tags?;
    let name = tags.get("name")?.to_string();
    let dog_policy = map_dog_policy(tags.get("dog"));
    if dog_policy == DogPolicy::NotAllowed {
        return None;
    }
    let dog_notes = tags.get("dog").and_then(|value| match value.as_str() {
        "leashed" | "on_leash" | "conditional" => {
            Some("Dogs must be leashed or have restrictions.".to_string())
        }
        _ => None,
    });

    let surface = tags
        .get("surface")
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());
    let distance_km = element
        .geometry
        .as_ref()
        .map(|points| compute_distance_km(points))
        .unwrap_or(0.0);

    let difficulty = crate::map_difficulty(tags.get("sac_scale"), distance_km);
    let location = tags
        .get("addr:city")
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());
    let map_url = format!("https://www.openstreetmap.org/way/{}", element.id);

    let lat = element
        .center
        .as_ref()
        .map(|c| c.lat)
        .or_else(|| {
            element.geometry.as_ref().and_then(|pts| {
                if pts.is_empty() {
                    None
                } else {
                    Some(pts.iter().map(|p| p.lat).sum::<f64>() / pts.len() as f64)
                }
            })
        })
        .unwrap_or(0.0);
    let lon = element
        .center
        .as_ref()
        .map(|c| c.lon)
        .or_else(|| {
            element.geometry.as_ref().and_then(|pts| {
                if pts.is_empty() {
                    None
                } else {
                    Some(pts.iter().map(|p| p.lon).sum::<f64>() / pts.len() as f64)
                }
            })
        })
        .unwrap_or(0.0);

    Some(Trail {
        id: format!("osm-{}", element.id),
        name,
        provider: Provider::OpenStreetMap,
        location,
        distance_km,
        elevation_m: tags.get("ele").and_then(|value| value.parse::<f32>().ok()),
        difficulty,
        dog_policy,
        dog_notes,
        surface,
        map_url,
        lat,
        lon,
        line_bbox: Bbox {
            min_lat: lat,
            min_lon: lon,
            max_lat: lat,
            max_lon: lon,
        },
    })
}

fn map_dog_policy(value: Option<&String>) -> DogPolicy {
    match value.map(|value| value.as_str()) {
        Some("yes") => DogPolicy::Allowed,
        Some("leashed") | Some("on_leash") | Some("conditional") => DogPolicy::Partial,
        Some("no") => DogPolicy::NotAllowed,
        _ => DogPolicy::NotAllowed,
    }
}

fn compute_distance_km(points: &[OverpassPoint]) -> f32 {
    if points.len() < 2 {
        return 0.0;
    }
    let mut total = 0.0;
    for window in points.windows(2) {
        total += haversine_km(window[0].lat, window[0].lon, window[1].lat, window[1].lon);
    }
    total as f32
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let radius = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();

    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    radius * c
}
