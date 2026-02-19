use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use dogtrails::{filter_trails, ProviderInfo, TrailQuery, TrailService};

#[derive(Clone)]
struct AppState {
    service: Arc<TrailService>,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

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

    let doc_api_key = std::env::var("DOC_API_KEY").unwrap();
    let service = TrailService::new(overpass_urls, doc_api_key)
        .expect("failed to create trail service");
    let state = AppState {
        service: Arc::new(service),
    };

    let app = Router::new()
        .route("/api/trails", get(get_trails))
        .route("/api/providers", get(get_providers))
        .nest_service(
            "/",
            ServeDir::new("frontend/dist").append_index_html_on_directories(true),
        )
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
) -> Result<Json<Vec<dogtrails::Trail>>, (StatusCode, String)> {
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
