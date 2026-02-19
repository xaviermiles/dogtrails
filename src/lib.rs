use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Easy,
    Moderate,
    Hard,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DogPolicy {
    Allowed,
    Partial,
    NotAllowed,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Provider {
    DOC,
    OpenStreetMap,
    AllTrails,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::DOC => write!(f, "DOC"),
            Provider::OpenStreetMap => write!(f, "OpenStreetMap"),
            Provider::AllTrails => write!(f, "AllTrails"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Trail {
    pub id: String,
    pub name: String,
    pub provider: Provider,
    pub location: String,
    pub distance_km: f32,
    pub elevation_m: u32,
    pub difficulty: Difficulty,
    pub dog_policy: DogPolicy,
    pub dog_notes: Option<String>,
    pub surface: String,
    pub map_url: String,
    pub lat: f64,
    pub lon: f64,
    #[serde(skip)]
    pub line_bbox: Bbox,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DogFilter {
    AllowedOnly,
    AllowedOrPartial,
    Any,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Easy,
    Steady,
    Hard,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Length {
    Short,
    Medium,
    Long,
}

#[derive(Clone, Deserialize, Default)]
pub struct TrailQuery {
    pub min_km: Option<f32>,
    pub max_km: Option<f32>,
    pub difficulty: Option<Difficulty>,
    pub dog: Option<DogFilter>,
    pub effort: Option<Effort>,
    pub length: Option<Length>,
    pub min_lat: Option<f64>,
    pub min_lon: Option<f64>,
    pub max_lat: Option<f64>,
    pub max_lon: Option<f64>,
}

#[derive(Clone, Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub api_status: String,
    pub notes: String,
    pub website: String,
}

impl ProviderInfo {
    pub fn default_providers() -> Vec<Self> {
        vec![
            ProviderInfo {
                name: "NZ Department of Conservation (DOC)".to_string(),
                api_status: "Public API (key required)".to_string(),
                notes: "Set DOC_API_KEY to enable DOC track data.".to_string(),
                website: "https://www.doc.govt.nz".to_string(),
            },
            ProviderInfo {
                name: "AllTrails".to_string(),
                api_status: "No public trails API confirmed".to_string(),
                notes: "Consider user-provided links or approved data feeds; avoid scraping without permission.".to_string(),
                website: "https://www.alltrails.com".to_string(),
            },
            ProviderInfo {
                name: "OpenStreetMap Overpass".to_string(),
                api_status: "Public API".to_string(),
                notes: "Uses public OSM data with dog access tags when present.".to_string(),
                website: "https://overpass-api.de".to_string(),
            },
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bbox {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

impl Default for Bbox {
    fn default() -> Self {
        Self {
            min_lat: -43.60,
            min_lon: 172.50,
            max_lat: -43.45,
            max_lon: 172.77,
        }
    }
}

impl Bbox {
    pub fn from_query(query: &TrailQuery) -> Option<Self> {
        Some(Self {
            min_lat: query.min_lat?,
            min_lon: query.min_lon?,
            max_lat: query.max_lat?,
            max_lon: query.max_lon?,
        })
    }
}

#[derive(Debug)]
pub struct TrailError(pub String);

impl std::fmt::Display for TrailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TrailError {}

pub struct TrailService {
    client: reqwest::Client,
    overpass_urls: Vec<String>,
    overpass_cache: RwLock<Option<OverpassCacheEntry>>,
    overpass_semaphore: tokio::sync::Semaphore,
    doc_cache: RwLock<Option<DocCacheEntry>>,
    doc_api_key: Option<String>,
}

struct OverpassCacheEntry {
    fetched_at: Instant,
    bbox: Bbox,
    trails: Vec<Trail>,
}

struct DocCacheEntry {
    fetched_at: Instant,
    trails: Vec<Trail>,
}

impl TrailService {
    pub fn new(overpass_urls: Vec<String>, doc_api_key: Option<String>) -> Result<Self, TrailError> {
        let client = reqwest::Client::builder()
            .user_agent("stravata/0.1 (https://example.local)")
            .build()
            .map_err(|err| TrailError(format!("failed to build http client: {err}")))?;
        Ok(Self {
            client,
            overpass_urls,
            overpass_cache: RwLock::new(None),
            overpass_semaphore: tokio::sync::Semaphore::new(1),
            doc_cache: RwLock::new(None),
            doc_api_key,
        })
    }

    pub async fn fetch_trails(&self, query: &TrailQuery) -> Result<Vec<Trail>, TrailError> {
        let bbox = Bbox::from_query(query).unwrap_or_default();
        let overpass_trails = self.fetch_overpass_cached(bbox).await?;
        let mut combined = overpass_trails;

        if let Some(api_key) = self.doc_api_key.as_ref() {
            match self.fetch_doc_cached(api_key, bbox).await {
                Ok(mut doc_trails) => combined.append(&mut doc_trails),
                Err(err) => {
                    tracing::warn!("DOC fetch failed: {}", err);
                }
            }
        }

        Ok(combined)
    }

    async fn fetch_overpass_cached(&self, bbox: Bbox) -> Result<Vec<Trail>, TrailError> {
        let ttl = Duration::from_secs(600);

        if let Some(cached) = self.overpass_cache.read().await.as_ref() {
            if cached.bbox == bbox && cached.fetched_at.elapsed() < ttl {
                return Ok(cached.trails.clone());
            }
        }

        // Only allow one in-flight Overpass request at a time
        let permit = match self.overpass_semaphore.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                // Another request is in-flight; serve stale cache if available
                if let Some(cached) = self.overpass_cache.read().await.as_ref() {
                    tracing::debug!("overpass request in-flight, serving cached data");
                    return Ok(cached.trails.clone());
                }
                // No cache at all; wait for the permit
                self.overpass_semaphore.acquire().await
                    .map_err(|_| TrailError("semaphore closed".to_string()))?
            }
        };

        // Re-check cache after acquiring permit (another request may have just finished)
        if let Some(cached) = self.overpass_cache.read().await.as_ref() {
            if cached.bbox == bbox && cached.fetched_at.elapsed() < ttl {
                drop(permit);
                return Ok(cached.trails.clone());
            }
        }

        let trails = fetch_overpass_with_fallback(&self.client, &self.overpass_urls, bbox).await?;
        let mut cache = self.overpass_cache.write().await;
        *cache = Some(OverpassCacheEntry {
            fetched_at: Instant::now(),
            bbox,
            trails: trails.clone(),
        });
        drop(permit);
        Ok(trails)
    }

    async fn fetch_doc_cached(&self, api_key: &str, bbox: Bbox) -> Result<Vec<Trail>, TrailError> {
        let ttl = Duration::from_secs(60 * 60 * 12);

        if let Some(cached) = self.doc_cache.read().await.as_ref() {
            if cached.fetched_at.elapsed() < ttl {
                return Ok(filter_doc_by_bbox(&cached.trails, bbox));
            }
        }

        // Fetch all DOC trails (no bbox filter) and cache globally
        let trails = fetch_doc_tracks_all(&self.client, api_key).await?;
        let mut cache = self.doc_cache.write().await;
        *cache = Some(DocCacheEntry {
            fetched_at: Instant::now(),
            trails: trails.clone(),
        });

        Ok(filter_doc_by_bbox(&trails, bbox))
    }
}

pub fn filter_trails(trails: &[Trail], query: &TrailQuery) -> Vec<Trail> {
    let dog_filter = query.dog.clone().unwrap_or(DogFilter::AllowedOrPartial);
    let range = derive_distance_range(query);
    let effort = query.effort.clone();

    let mut matches: Vec<(Trail, f32)> = trails
        .iter()
        .cloned()
        .filter(|trail| dog_policy_allows(trail, &dog_filter))
        .filter(|trail| match query.difficulty {
            Some(ref difficulty) => &trail.difficulty == difficulty,
            None => true,
        })
        .filter(|trail| within_distance(trail.distance_km, &range))
        .map(|trail| {
            let score = score_trail(&trail, &range, effort.as_ref());
            (trail, score)
        })
        .collect();

    matches.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    matches
        .into_iter()
        .map(|(trail, _)| trail)
        .collect()
}

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

async fn fetch_overpass_with_fallback(
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

const DOC_TRACKS_URL: &str = "https://api.doc.govt.nz/v1/tracks?coordinates=wgs84";

async fn fetch_doc_tracks_all(
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

    let candidates: Vec<(String, Value)> = items
        .into_iter()
        .filter_map(|item| {
            let track_id = extract_doc_id(&item)?;
            Some((track_id, item))
        })
        .collect();

    tracing::info!("DOC: {} tracks with valid IDs", candidates.len());

    // Fetch details in parallel with a concurrency limit
    const MAX_CONCURRENT: usize = 5;
    let mut trails = Vec::new();
    for chunk in candidates.chunks(MAX_CONCURRENT) {
        let mut set = tokio::task::JoinSet::new();
        for (track_id, item) in chunk.iter().cloned() {
            let client = client.clone();
            let api_key = api_key.to_string();
            set.spawn(async move {
                let detail = fetch_doc_detail(&client, &api_key, &track_id).await;
                (item, track_id, detail)
            });
        }
        while let Some(result) = set.join_next().await {
            if let Ok((item, track_id, detail_result)) = result {
                match detail_result {
                    Ok(detail) => {
                        let line_bbox = extract_line_bbox(&item)
                            .or_else(|| extract_line_bbox(&detail));
                        if let Some(mut trail) = map_doc_track_no_bbox(&item, &detail) {
                            if let Some(lb) = line_bbox {
                                trail.line_bbox = lb;
                            }
                            trails.push(trail);
                        }
                    }
                    Err(err) => {
                        tracing::warn!("DOC detail fetch failed for {}: {}", track_id, err);
                    }
                }
            }
        }
    }

    tracing::info!("DOC: {} trails after mapping", trails.len());
    Ok(trails)
}

async fn fetch_doc_detail(
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

fn map_doc_track_no_bbox(summary: &Value, detail: &Value) -> Option<Trail> {
    let name = doc_string_any(detail, summary, &["name", "trackName", "title"]) ?;

    let (dog_policy, dog_notes) = doc_dog_policy(detail, summary);

    let location = doc_string_any(
        detail,
        summary,
        &["locationString", "locationArray", "location", "region", "district", "place", "area"],
    )
    .unwrap_or_else(|| "New Zealand".to_string());

    let surface = doc_string_any(detail, summary, &["surface", "trackSurface", "terrain"])
        .unwrap_or_else(|| "Unknown".to_string());

    let distance_km = doc_distance_km(detail, summary).unwrap_or(0.0);
    let elevation_m = doc_number_any(detail, summary, &["elevationGain", "totalAscent", "ascent", "elevation"]) 
        .unwrap_or(0.0)
        .round() as u32;

    let difficulty = doc_difficulty(detail, summary).unwrap_or_else(|| map_difficulty(None, distance_km));

    let map_url = doc_string_any(detail, summary, &["staticLink", "url", "webUrl", "docUrl", "link"])
        .unwrap_or_else(|| "https://www.doc.govt.nz".to_string());

    let id = extract_doc_id(detail)
        .or_else(|| extract_doc_id(summary))
        .unwrap_or_else(|| name.to_lowercase().replace(' ', "-"));

    let (trail_lat, trail_lon) = doc_lat_lon(detail, summary).unwrap_or((0.0, 0.0));

    Some(Trail {
        id: format!("doc-{}", id),
        name,
        provider: Provider::DOC,
        location,
        distance_km,
        elevation_m,
        difficulty,
        dog_policy,
        dog_notes,
        surface,
        map_url,
        lat: trail_lat,
        lon: trail_lon,
        line_bbox: Bbox { min_lat: trail_lat, min_lon: trail_lon, max_lat: trail_lat, max_lon: trail_lon },
    })
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

fn doc_string_any(primary: &Value, secondary: &Value, keys: &[&str]) -> Option<String> {
    doc_string(primary, keys).or_else(|| doc_string(secondary, keys))
}

fn doc_number_any(primary: &Value, secondary: &Value, keys: &[&str]) -> Option<f64> {
    doc_number(primary, keys).or_else(|| doc_number(secondary, keys))
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

fn doc_distance_km(primary: &Value, secondary: &Value) -> Option<f32> {
    let raw = doc_number_any(primary, secondary, &["distance", "distanceKm", "length", "trackLength"])?;
    let text = doc_string_any(primary, secondary, &["distance", "distanceKm", "length", "trackLength"]);
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

fn doc_difficulty(primary: &Value, secondary: &Value) -> Option<Difficulty> {
    let value = doc_string_any(primary, secondary, &["difficulty", "grade", "trackGrade", "walkTrackCategory"])?;
    let lower = value.to_lowercase();
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

fn doc_dog_policy(primary: &Value, secondary: &Value) -> (DogPolicy, Option<String>) {
    let allowed = doc_bool_any(primary, secondary, &["dogsAllowed", "dogAllowed"]);
    let on_lead = doc_bool_any(primary, secondary, &["dogsAllowedOnLead", "dogsOnLead"]);
    if let Some(false) = allowed {
        return (DogPolicy::NotAllowed, Some("Dogs are not permitted.".to_string()));
    }
    if let Some(true) = allowed {
        if let Some(true) = on_lead {
            return (DogPolicy::Partial, Some("Dogs must be on a lead.".to_string()));
        }
        return (DogPolicy::Allowed, None);
    }

    if let Some(text) = doc_string_any(primary, secondary, &["dogsAllowed", "dogAccess", "dogs", "dogRules"]) {
        let lower = text.to_lowercase();
        if lower.contains("no") && lower.contains("dog") {
            return (DogPolicy::NotAllowed, Some(text));
        }
        if lower.contains("lead") || lower.contains("leash") || lower.contains("controlled") {
            return (DogPolicy::Partial, Some(text));
        }
        return (DogPolicy::Allowed, Some(text));
    }

    (
        DogPolicy::Partial,
        Some("Check the DOC track page for dog access details.".to_string()),
    )
}

fn doc_bool_any(primary: &Value, secondary: &Value, keys: &[&str]) -> Option<bool> {
    doc_bool(primary, keys).or_else(|| doc_bool(secondary, keys))
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

fn doc_lat_lon(primary: &Value, secondary: &Value) -> Option<(f64, f64)> {
    extract_lat_lon(primary).or_else(|| extract_lat_lon(secondary))
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
    a.min_lat <= b.max_lat && a.max_lat >= b.min_lat
        && a.min_lon <= b.max_lon && a.max_lon >= b.min_lon
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
        Some(Bbox { min_lat, min_lon, max_lat, max_lon })
    } else {
        None
    }
}

/// Filter DOC trails: include if the track's line bbox intersects the view.
fn filter_doc_by_bbox(trails: &[Trail], view: Bbox) -> Vec<Trail> {
    trails
        .iter()
        .filter(|trail| bbox_intersects(view, trail.line_bbox))
        .cloned()
        .collect()
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
            tracing::warn!("overpass 429, retrying in {:?} (attempt {}/{})", delay, attempt, max_retries);
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
            .filter_map(|element| map_overpass_element(element))
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
        "leashed" | "on_leash" | "conditional" => Some("Dogs must be leashed or have restrictions.".to_string()),
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

    let difficulty = map_difficulty(tags.get("sac_scale"), distance_km);
    let location = tags
        .get("addr:city")
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());
    let map_url = format!("https://www.openstreetmap.org/way/{}", element.id);

    let lat = element.center.as_ref().map(|c| c.lat)
        .or_else(|| element.geometry.as_ref().and_then(|pts| {
            if pts.is_empty() { None } else {
                Some(pts.iter().map(|p| p.lat).sum::<f64>() / pts.len() as f64)
            }
        })).unwrap_or(0.0);
    let lon = element.center.as_ref().map(|c| c.lon)
        .or_else(|| element.geometry.as_ref().and_then(|pts| {
            if pts.is_empty() { None } else {
                Some(pts.iter().map(|p| p.lon).sum::<f64>() / pts.len() as f64)
            }
        })).unwrap_or(0.0);

    Some(Trail {
        id: format!("osm-{}", element.id),
        name,
        provider: Provider::OpenStreetMap,
        location,
        distance_km,
        elevation_m: tags
            .get("ele")
            .and_then(|value| value.parse::<f32>().ok())
            .map(|value| value as u32)
            .unwrap_or(0),
        difficulty,
        dog_policy,
        dog_notes,
        surface,
        map_url,
        lat,
        lon,
        line_bbox: Bbox { min_lat: lat, min_lon: lon, max_lat: lat, max_lon: lon },
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

fn map_difficulty(sac_scale: Option<&String>, distance_km: f32) -> Difficulty {
    if let Some(scale) = sac_scale {
        return match scale.as_str() {
            "hiking" => Difficulty::Easy,
            "mountain_hiking" => Difficulty::Moderate,
            "demanding_mountain_hiking" | "alpine_hiking" => Difficulty::Hard,
            _ => Difficulty::Moderate,
        };
    }

    if distance_km <= 6.0 {
        Difficulty::Easy
    } else if distance_km <= 14.0 {
        Difficulty::Moderate
    } else {
        Difficulty::Hard
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

    let a = (dlat / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    radius * c
}

fn dog_policy_allows(trail: &Trail, filter: &DogFilter) -> bool {
    match filter {
        DogFilter::AllowedOnly => trail.dog_policy == DogPolicy::Allowed,
        DogFilter::AllowedOrPartial => {
            trail.dog_policy == DogPolicy::Allowed || trail.dog_policy == DogPolicy::Partial
        }
        DogFilter::Any => true,
    }
}

fn derive_distance_range(query: &TrailQuery) -> (Option<f32>, Option<f32>, Option<f32>) {
    let min_km = query.min_km;
    let max_km = query.max_km;
    if min_km.is_some() || max_km.is_some() {
        let target = min_km
            .zip(max_km)
            .map(|(min, max)| (min + max) / 2.0);
        return (min_km, max_km, target);
    }

    match query.length.clone().unwrap_or(Length::Medium) {
        Length::Short => (Some(2.0), Some(6.0), Some(4.0)),
        Length::Medium => (Some(6.0), Some(12.0), Some(9.0)),
        Length::Long => (Some(12.0), Some(24.0), Some(16.0)),
    }
}

fn within_distance(distance_km: f32, range: &(Option<f32>, Option<f32>, Option<f32>)) -> bool {
    if distance_km == 0.0 {
        return true;
    }
    let (min_km, max_km, _) = range;
    if let Some(min) = min_km {
        if distance_km < *min {
            return false;
        }
    }
    if let Some(max) = max_km {
        if distance_km > *max {
            return false;
        }
    }
    true
}

fn score_trail(trail: &Trail, range: &(Option<f32>, Option<f32>, Option<f32>), effort: Option<&Effort>) -> f32 {
    let target = range.2.unwrap_or(trail.distance_km);
    let distance_penalty = (trail.distance_km - target).abs();

    let effort_penalty = match effort {
        Some(Effort::Easy) => difficulty_penalty(&trail.difficulty, &Difficulty::Easy),
        Some(Effort::Steady) => difficulty_penalty(&trail.difficulty, &Difficulty::Moderate),
        Some(Effort::Hard) => difficulty_penalty(&trail.difficulty, &Difficulty::Hard),
        None => 0.5,
    };

    let elevation_penalty = trail.elevation_m as f32 / 600.0;
    distance_penalty + effort_penalty * 2.0 + elevation_penalty
}

fn difficulty_penalty(actual: &Difficulty, preferred: &Difficulty) -> f32 {
    let actual_score = difficulty_rank(actual);
    let preferred_score = difficulty_rank(preferred);
    (actual_score - preferred_score).abs() as f32
}

fn difficulty_rank(difficulty: &Difficulty) -> i32 {
    match difficulty {
        Difficulty::Easy => 1,
        Difficulty::Moderate => 2,
        Difficulty::Hard => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trails() -> Vec<Trail> {
        vec![
            Trail {
                id: "t1".to_string(),
                name: "River Loop".to_string(),
                provider: Provider::DOC,
                location: "Wellington".to_string(),
                distance_km: 5.0,
                elevation_m: 120,
                difficulty: Difficulty::Easy,
                dog_policy: DogPolicy::Allowed,
                dog_notes: None,
                surface: "Gravel".to_string(),
                map_url: "https://www.doc.govt.nz".to_string(),
                lat: -41.3,
                lon: 174.7,
                line_bbox: Bbox { min_lat: -41.3, min_lon: 174.7, max_lat: -41.3, max_lon: 174.7 },
            },
            Trail {
                id: "t2".to_string(),
                name: "Forest Ridge".to_string(),
                provider: Provider::AllTrails,
                location: "Auckland".to_string(),
                distance_km: 12.0,
                elevation_m: 520,
                difficulty: Difficulty::Hard,
                dog_policy: DogPolicy::NotAllowed,
                dog_notes: Some("Dog-free section after 2km".to_string()),
                surface: "Dirt".to_string(),
                map_url: "https://www.alltrails.com".to_string(),
                lat: -36.8,
                lon: 174.7,
                line_bbox: Bbox { min_lat: -36.8, min_lon: 174.7, max_lat: -36.8, max_lon: 174.7 },
            },
        ]
    }

    #[test]
    fn filters_dog_allowed_by_default() {
        let trails = sample_trails();
        let query = TrailQuery {
            length: Some(Length::Short),
            ..TrailQuery::default()
        };
        let results = filter_trails(&trails, &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "t1");
    }

    #[test]
    fn allows_any_dog_policy_when_requested() {
        let trails = sample_trails();
        let query = TrailQuery {
            dog: Some(DogFilter::Any),
            min_km: Some(0.0),
            max_km: Some(20.0),
            ..TrailQuery::default()
        };
        let results = filter_trails(&trails, &query);
        assert_eq!(results.len(), 2);
    }
}
