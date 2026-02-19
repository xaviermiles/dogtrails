use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    response::Html,
    Json, Router,
};
use serde::Deserialize;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use stravata::{filter_trails, Bbox, ProviderInfo, TrailQuery, TrailService};

#[derive(Clone)]
struct AppState {
    service: Arc<TrailService>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let overpass_urls = std::env::var("OVERPASS_URL")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(|entry| entry.trim().to_string())
                .filter(|entry| !entry.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|entries| !entries.is_empty())
        .unwrap_or_else(|| {
            vec![
                "https://overpass-api.de/api/interpreter".to_string(),
                "https://overpass.kumi.systems/api/interpreter".to_string(),
                "https://overpass.nchc.org.tw/api/interpreter".to_string(),
            ]
        });
    let service = TrailService::new(overpass_urls).expect("failed to create trail service");
    let state = AppState {
        service: Arc::new(service),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/api/trails", get(get_trails))
        .route("/api/providers", get(get_providers))
        .nest_service("/static", ServeDir::new("public"))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3000);
    let address = SocketAddr::from(([127, 0, 0, 1], port));

    tracing::info!("listening on http://{}", address);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind address");
    axum::serve(listener, app)
        .await
        .expect("server error");
}

async fn get_trails(
    State(state): State<AppState>,
    Query(query): Query<TrailQuery>,
) -> Result<Json<Vec<stravata::Trail>>, (StatusCode, String)> {
    let trails = state
        .service
        .fetch_trails(&query)
        .await
        .map_err(|err| (StatusCode::BAD_GATEWAY, err.to_string()))?;
    let filtered = filter_trails(&trails, &query);
    Ok(Json(filtered))
}

async fn get_providers() -> Json<Vec<ProviderInfo>> {
    Json(ProviderInfo::default_providers())
}

#[derive(Deserialize, Default, Clone)]
struct PageQuery {
        region: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    min_km: Option<f32>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    max_km: Option<f32>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    difficulty: Option<stravata::Difficulty>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    dog: Option<stravata::DogFilter>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    effort: Option<stravata::Effort>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    length: Option<stravata::Length>,
        #[serde(default, deserialize_with = "empty_string_as_none")]
        max_results: Option<usize>,
        #[serde(default, deserialize_with = "empty_string_as_none")]
        min_lat: Option<f64>,
        #[serde(default, deserialize_with = "empty_string_as_none")]
        min_lon: Option<f64>,
        #[serde(default, deserialize_with = "empty_string_as_none")]
        max_lat: Option<f64>,
        #[serde(default, deserialize_with = "empty_string_as_none")]
        max_lon: Option<f64>,
}

impl PageQuery {
        fn to_trail_query(&self) -> TrailQuery {
                let mut query = TrailQuery {
                        min_km: self.min_km,
                        max_km: self.max_km,
                        difficulty: self.difficulty.clone(),
                        dog: self.dog.clone(),
                        effort: self.effort.clone(),
                        length: self.length.clone(),
                        max_results: self.max_results,
                        min_lat: self.min_lat,
                        min_lon: self.min_lon,
                        max_lat: self.max_lat,
                        max_lon: self.max_lon,
                };

                if Bbox::from_query(&query).is_none() {
                        if let Some(region) = self.region.as_deref() {
                                if let Some(bbox) = region_bbox(region) {
                                        query.min_lat = Some(bbox.min_lat);
                                        query.min_lon = Some(bbox.min_lon);
                                        query.max_lat = Some(bbox.max_lat);
                                        query.max_lon = Some(bbox.max_lon);
                                }
                        }
                }
                query
        }
}

async fn index(
        State(state): State<AppState>,
        Query(query): Query<PageQuery>,
) -> Html<String> {
        let trail_query = query.to_trail_query();
        let providers = ProviderInfo::default_providers();

        let (trails, error_message) = match state.service.fetch_trails(&trail_query).await {
                Ok(trails) => (filter_trails(&trails, &trail_query), None),
                Err(err) => (Vec::new(), Some(err.to_string())),
        };

        Html(render_page(&query, &trails, &providers, error_message.as_deref()))
}

fn render_page(
        query: &PageQuery,
        trails: &[stravata::Trail],
        providers: &[ProviderInfo],
        error_message: Option<&str>,
) -> String {
        let region = query.region.as_deref().unwrap_or("wellington");
        let difficulty = difficulty_value(query.difficulty.as_ref());
        let dog = dog_filter_value(query.dog.as_ref());
        let effort = effort_value(query.effort.as_ref());
        let length = length_value(query.length.as_ref());

        let results = if trails.is_empty() {
                if let Some(error) = error_message {
                        format!(
                                "<div class=\"warning\">Could not load live trails. {}</div>",
                                html_escape(error)
                        )
                } else {
                        "<div class=\"warning\">No trails matched your filters. Try a wider region.</div>"
                                .to_string()
                }
        } else {
                trails
                        .iter()
                        .map(render_trail)
                        .collect::<Vec<_>>()
                        .join("")
        };

        let provider_items = providers
                .iter()
                .map(|provider| {
                        format!(
                                "<li><strong>{}</strong><br /><span>{}</span><br /><em>{}</em><br /><a href=\"{}\" target=\"_blank\" rel=\"noreferrer\">{}</a></li>",
                                html_escape(&provider.name),
                                html_escape(&provider.api_status),
                                html_escape(&provider.notes),
                                html_escape(&provider.website),
                                html_escape(&provider.website)
                        )
                })
                .collect::<Vec<_>>()
                .join("");

        format!(
                "<!doctype html>
<html lang=\"en\">
    <head>
        <meta charset=\"UTF-8\" />
        <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />
        <title>Stravata Trails</title>
        <link rel=\"stylesheet\" href=\"/static/styles.css\" />
    </head>
    <body>
        <div class=\"app\">
            <header>
                <div>
                    <p class=\"eyebrow\">Stravata</p>
                    <h1>Trail recommendations for you + dog</h1>
                    <p class=\"subhead\">Server-rendered, no JavaScript required.</p>
                </div>
            </header>

            <main>
                <section class=\"card form-card\">
                    <h2>Plan your run</h2>
                    <form method=\"get\" action=\"/\">
                        <div class=\"grid\">
                            <label>
                                Region
                                <select name=\"region\">
                                    <option value=\"wellington\" {wellington}>Wellington</option>
                                    <option value=\"auckland\" {auckland}>Auckland</option>
                                    <option value=\"queenstown\" {queenstown}>Queenstown</option>
                                    <option value=\"christchurch\" {christchurch}>Christchurch</option>
                                </select>
                            </label>
                            <label>
                                Effort
                                <select name=\"effort\">
                                    <option value=\"easy\" {effort_easy}>Easy</option>
                                    <option value=\"steady\" {effort_steady}>Steady</option>
                                    <option value=\"hard\" {effort_hard}>Hard</option>
                                </select>
                            </label>
                            <label>
                                Length
                                <select name=\"length\">
                                    <option value=\"short\" {length_short}>Short (2-6 km)</option>
                                    <option value=\"medium\" {length_medium}>Medium (6-12 km)</option>
                                    <option value=\"long\" {length_long}>Long (12-24 km)</option>
                                </select>
                            </label>
                            <label>
                                Dog access
                                <select name=\"dog\">
                                    <option value=\"allowed_only\" {dog_allowed_only}>Dogs allowed only</option>
                                    <option value=\"allowed_or_partial\" {dog_allowed_or_partial}>Allowed or partial (with notes)</option>
                                    <option value=\"any\" {dog_any}>Show all (include no-dog)</option>
                                </select>
                            </label>
                            <label>
                                Difficulty
                                <select name=\"difficulty\">
                                    <option value=\"\" {difficulty_any}>Any</option>
                                    <option value=\"easy\" {difficulty_easy}>Easy</option>
                                    <option value=\"moderate\" {difficulty_moderate}>Moderate</option>
                                    <option value=\"hard\" {difficulty_hard}>Hard</option>
                                </select>
                            </label>
                            <label>
                                Min distance (km)
                                <input type=\"number\" name=\"min_km\" min=\"1\" step=\"0.5\" value=\"{min_km}\" />
                            </label>
                            <label>
                                Max distance (km)
                                <input type=\"number\" name=\"max_km\" min=\"1\" step=\"0.5\" value=\"{max_km}\" />
                            </label>
                            <label>
                                Min latitude
                                <input type=\"number\" name=\"min_lat\" step=\"0.0001\" value=\"{min_lat}\" />
                            </label>
                            <label>
                                Min longitude
                                <input type=\"number\" name=\"min_lon\" step=\"0.0001\" value=\"{min_lon}\" />
                            </label>
                            <label>
                                Max latitude
                                <input type=\"number\" name=\"max_lat\" step=\"0.0001\" value=\"{max_lat}\" />
                            </label>
                            <label>
                                Max longitude
                                <input type=\"number\" name=\"max_lon\" step=\"0.0001\" value=\"{max_lon}\" />
                            </label>
                        </div>
                        <button type=\"submit\">Find trails</button>
                    </form>
                    <div class=\"integration\">
                        <h3>Fitness integrations (coming soon)</h3>
                        <p>Connect Strava or Garmin to calibrate recommendations to your training history.</p>
                        <div class=\"integration-buttons\">
                            <button type=\"button\" class=\"ghost\">Connect Strava</button>
                            <button type=\"button\" class=\"ghost\">Connect Garmin</button>
                        </div>
                    </div>
                </section>

                <section class=\"card\">
                    <div class=\"results-header\">
                        <h2>Suggested routes</h2>
                        <span>{result_count}</span>
                    </div>
                    <div class=\"results\">{results}</div>
                </section>

                <section class=\"card\">
                    <h2>Provider notes</h2>
                    <ul class=\"providers\">{providers}</ul>
                    <p class=\"note\">We use live OpenStreetMap data today. DOC/AllTrails integration requires an approved API.</p>
                </section>
            </main>
        </div>
    </body>
</html>
",
                wellington = selected(region == "wellington"),
                auckland = selected(region == "auckland"),
                queenstown = selected(region == "queenstown"),
                christchurch = selected(region == "christchurch"),
                effort_easy = selected(effort == "easy"),
                effort_steady = selected(effort == "steady"),
                effort_hard = selected(effort == "hard"),
                length_short = selected(length == "short"),
                length_medium = selected(length == "medium"),
                length_long = selected(length == "long"),
                dog_allowed_only = selected(dog == "allowed_only"),
                dog_allowed_or_partial = selected(dog == "allowed_or_partial"),
                dog_any = selected(dog == "any"),
                difficulty_any = selected(difficulty.is_empty()),
                difficulty_easy = selected(difficulty == "easy"),
                difficulty_moderate = selected(difficulty == "moderate"),
                difficulty_hard = selected(difficulty == "hard"),
                min_km = value_or_empty(query.min_km),
                max_km = value_or_empty(query.max_km),
                min_lat = value_or_empty(query.min_lat),
                min_lon = value_or_empty(query.min_lon),
                max_lat = value_or_empty(query.max_lat),
                max_lon = value_or_empty(query.max_lon),
                result_count = format!("{} route{}", trails.len(), if trails.len() == 1 { "" } else { "s" }),
                results = results,
                providers = provider_items,
        )
}

fn region_bbox(region: &str) -> Option<Bbox> {
        match region {
                "wellington" => Some(Bbox {
                        min_lat: -41.35,
                        min_lon: 174.72,
                        max_lat: -41.24,
                        max_lon: 174.82,
                }),
                "auckland" => Some(Bbox {
                        min_lat: -36.93,
                        min_lon: 174.63,
                        max_lat: -36.77,
                        max_lon: 174.84,
                }),
                "queenstown" => Some(Bbox {
                        min_lat: -45.08,
                        min_lon: 168.56,
                        max_lat: -44.95,
                        max_lon: 168.79,
                }),
                "christchurch" => Some(Bbox {
                        min_lat: -43.60,
                        min_lon: 172.50,
                        max_lat: -43.45,
                        max_lon: 172.77,
                }),
                _ => None,
        }
}

fn render_trail(trail: &stravata::Trail) -> String {
        let warning = if trail.dog_policy != stravata::DogPolicy::Allowed {
                format!(
                        "<div class=\"warning\">{}</div>",
                        html_escape(
                                trail
                                        .dog_notes
                                        .as_deref()
                                        .unwrap_or("Dog access has restrictions.")
                        )
                )
        } else {
                String::new()
        };

        let distance_label = if trail.distance_km == 0.0 {
                "distance unknown".to_string()
        } else {
                format!("{:.1} km", trail.distance_km)
        };

        format!(
                "<article class=\"trail\">
                        <h3>{}</h3>
                        <div class=\"trail-meta\">
                            <span class=\"tag\">{}</span>
                            <span class=\"tag\">{}</span>
                            <span class=\"tag\">{} m gain</span>
                            <span class=\"tag\">{}</span>
                            <span class=\"tag\">{}</span>
                        </div>
                        <div class=\"trail-meta\">
                            <span class=\"tag\">Dog policy: {}</span>
                            <span class=\"tag\">Surface: {}</span>
                        </div>
                        <div>
                            <a href=\"{}\" target=\"_blank\" rel=\"noreferrer\">View map</a>
                        </div>
                        {}
                    </article>",
                html_escape(&trail.name),
                html_escape(&trail.location),
                html_escape(&distance_label),
                trail.elevation_m,
                format_label(&difficulty_value(Some(&trail.difficulty))),
                html_escape(&trail.provider),
                format_label(&dog_policy_value(&trail.dog_policy)),
                html_escape(&trail.surface),
                html_escape(&trail.map_url),
                warning
        )
}

fn selected(condition: bool) -> &'static str {
        if condition {
                "selected"
        } else {
                ""
        }
}

fn value_or_empty<T: std::fmt::Display>(value: Option<T>) -> String {
        value.map(|value| value.to_string()).unwrap_or_default()
}

fn empty_string_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let option = Option::<String>::deserialize(deserializer)?;
    match option.as_deref() {
        Some("") | None => Ok(None),
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            let parsed = T::deserialize(serde_json::Value::String(value.to_string()))
                .map_err(serde::de::Error::custom)?;
            Ok(Some(parsed))
        }
    }
}

fn difficulty_value(value: Option<&stravata::Difficulty>) -> String {
    match value {
        Some(stravata::Difficulty::Easy) => "easy".to_string(),
        Some(stravata::Difficulty::Moderate) => "moderate".to_string(),
        Some(stravata::Difficulty::Hard) => "hard".to_string(),
        None => String::new(),
    }
}

fn dog_filter_value(value: Option<&stravata::DogFilter>) -> String {
    match value {
        Some(stravata::DogFilter::AllowedOnly) => "allowed_only".to_string(),
        Some(stravata::DogFilter::AllowedOrPartial) => "allowed_or_partial".to_string(),
        Some(stravata::DogFilter::Any) => "any".to_string(),
        None => "allowed_or_partial".to_string(),
    }
}

fn effort_value(value: Option<&stravata::Effort>) -> String {
    match value {
        Some(stravata::Effort::Easy) => "easy".to_string(),
        Some(stravata::Effort::Steady) => "steady".to_string(),
        Some(stravata::Effort::Hard) => "hard".to_string(),
        None => "steady".to_string(),
    }
}

fn length_value(value: Option<&stravata::Length>) -> String {
    match value {
        Some(stravata::Length::Short) => "short".to_string(),
        Some(stravata::Length::Medium) => "medium".to_string(),
        Some(stravata::Length::Long) => "long".to_string(),
        None => "medium".to_string(),
    }
}

fn dog_policy_value(value: &stravata::DogPolicy) -> String {
    match value {
        stravata::DogPolicy::Allowed => "allowed".to_string(),
        stravata::DogPolicy::Partial => "partial".to_string(),
        stravata::DogPolicy::NotAllowed => "not_allowed".to_string(),
    }
}

fn format_label(value: &str) -> String {
    value.replace('_', " ")
}

fn html_escape(value: &str) -> String {
        value
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;")
}
