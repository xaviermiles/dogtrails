use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use stravata::{filter_trails, load_trails, ProviderInfo, TrailQuery};

#[derive(Clone)]
struct AppState {
    trails: Arc<Vec<stravata::Trail>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let trails = load_trails().expect("failed to load trails data");
    let state = AppState {
        trails: Arc::new(trails),
    };

    let app = Router::new()
        .route("/api/trails", get(get_trails))
        .route("/api/providers", get(get_providers))
        .nest_service("/", ServeDir::new("public").append_index_html_on_directories(true))
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
) -> Json<Vec<stravata::Trail>> {
    let filtered = filter_trails(&state.trails, &query);
    Json(filtered)
}

async fn get_providers() -> Json<Vec<ProviderInfo>> {
    Json(ProviderInfo::default_providers())
}
