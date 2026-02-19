mod doc;
mod overpass;

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
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
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::DOC => write!(f, "DOC"),
            Provider::OpenStreetMap => write!(f, "OpenStreetMap"),
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
    pub elevation_m: Option<f32>,
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

        let trails = overpass::fetch_overpass_with_fallback(&self.client, &self.overpass_urls, bbox).await?;
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
                return Ok(doc::filter_doc_by_bbox(&cached.trails, bbox));
            }
        }

        // Fetch all DOC trails (no bbox filter) and cache globally
        let trails = doc::fetch_doc_tracks_all(&self.client, api_key).await?;
        let mut cache = self.doc_cache.write().await;
        *cache = Some(DocCacheEntry {
            fetched_at: Instant::now(),
            trails: trails.clone(),
        });

        Ok(doc::filter_doc_by_bbox(&trails, bbox))
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

pub(crate) fn map_difficulty(sac_scale: Option<&String>, distance_km: f32) -> Difficulty {
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

    let elevation_penalty = trail.elevation_m.unwrap_or(0.0) / 600.0;
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
                elevation_m: Some(120.0),
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
                provider: Provider::OpenStreetMap,
                location: "Auckland".to_string(),
                distance_km: 12.0,
                elevation_m: Some(520.0),
                difficulty: Difficulty::Hard,
                dog_policy: DogPolicy::NotAllowed,
                dog_notes: Some("Dog-free section after 2km".to_string()),
                surface: "Dirt".to_string(),
                map_url: "https://www.openstreetmap.org/".to_string(),
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
