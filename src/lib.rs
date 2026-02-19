use serde::{Deserialize, Serialize};

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

#[derive(Clone, Serialize, Deserialize)]
pub struct Trail {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub location: String,
    pub distance_km: f32,
    pub elevation_m: u32,
    pub difficulty: Difficulty,
    pub dog_policy: DogPolicy,
    pub dog_notes: Option<String>,
    pub surface: String,
    pub map_url: String,
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
    pub max_results: Option<usize>,
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
                api_status: "No public trails API confirmed".to_string(),
                notes: "Use manual curation or scrape permitted data; prefer DOC open data when available.".to_string(),
                website: "https://www.doc.govt.nz".to_string(),
            },
            ProviderInfo {
                name: "AllTrails".to_string(),
                api_status: "No public trails API confirmed".to_string(),
                notes: "Consider user-provided links or approved data feeds; avoid scraping without permission.".to_string(),
                website: "https://www.alltrails.com".to_string(),
            },
        ]
    }
}

pub fn load_trails() -> Result<Vec<Trail>, serde_json::Error> {
    let data = include_str!("../data/trails.json");
    serde_json::from_str(data)
}

pub fn filter_trails(trails: &[Trail], query: &TrailQuery) -> Vec<Trail> {
    let dog_filter = query.dog.clone().unwrap_or(DogFilter::AllowedOrPartial);
    let max_results = query.max_results.unwrap_or(6);
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
        .take(max_results)
        .map(|(trail, _)| trail)
        .collect()
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
                provider: "DOC".to_string(),
                location: "Wellington".to_string(),
                distance_km: 5.0,
                elevation_m: 120,
                difficulty: Difficulty::Easy,
                dog_policy: DogPolicy::Allowed,
                dog_notes: None,
                surface: "Gravel".to_string(),
                map_url: "https://www.doc.govt.nz".to_string(),
            },
            Trail {
                id: "t2".to_string(),
                name: "Forest Ridge".to_string(),
                provider: "AllTrails".to_string(),
                location: "Auckland".to_string(),
                distance_km: 12.0,
                elevation_m: 520,
                difficulty: Difficulty::Hard,
                dog_policy: DogPolicy::NotAllowed,
                dog_notes: Some("Dog-free section after 2km".to_string()),
                surface: "Dirt".to_string(),
                map_url: "https://www.alltrails.com".to_string(),
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
