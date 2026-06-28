mod backend;
mod render;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use backend::Backend;
use eyre::WrapErr as _;
use std::sync::Arc;
use tracing::info;

struct State {
    backends: Vec<Backend>,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    let mut backends = Vec::new();

    if let (Ok(url), Ok(user), Ok(pass)) = (
        std::env::var("NAVIDROME_URL"),
        std::env::var("NAVIDROME_USER"),
        std::env::var("NAVIDROME_PASS"),
    ) {
        info!(url = %url, user = %user, "navidrome backend enabled");
        backends.push(Backend::navidrome(url, user, pass)?);
    }

    if let (Ok(client_id), Ok(client_secret), Ok(refresh_token)) = (
        std::env::var("SPOTIFY_CLIENT_ID"),
        std::env::var("SPOTIFY_CLIENT_SECRET"),
        std::env::var("SPOTIFY_REFRESH_TOKEN"),
    ) && !client_id.is_empty()
    {
        info!("spotify backend enabled");
        backends.push(Backend::spotify(client_id, client_secret, refresh_token)?);
    }

    if backends.is_empty() {
        eyre::bail!("no backends configured — set NAVIDROME_* or SPOTIFY_* env vars");
    }

    let state = Arc::new(State { backends });
    let app = Router::new()
        .route("/framebuffer", get(framebuffer_handler))
        .route("/play-pause", post(play_pause_handler))
        .route("/next", post(next_handler))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .wrap_err("failed to parse PORT")?;

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .wrap_err("failed to bind listener")?;

    info!(port, "server listening");
    axum::serve(listener, app).await.wrap_err("server error")?;

    Ok(())
}

async fn framebuffer_handler(
    axum::extract::State(state): axum::extract::State<Arc<State>>,
) -> impl IntoResponse {
    // query all backends concurrently so a slow or unreachable one doesn't add
    // its latency on top of the others. results preserve configured order.
    let results = futures::future::join_all(state.backends.iter().map(|b| b.now_playing())).await;

    for result in results {
        match result {
            Ok(Some(np)) => {
                info!(track = %np.track, artist = %np.artist, "serving now playing");
                let fb = render::render_now_playing(&np);
                return ([(header::CONTENT_TYPE, "application/octet-stream")], fb);
            }
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(error = %e, "backend error");
                continue;
            }
        }
    }

    info!("nothing playing");
    let fb = render::render_idle();
    ([(header::CONTENT_TYPE, "application/octet-stream")], fb)
}

async fn play_pause_handler(
    axum::extract::State(state): axum::extract::State<Arc<State>>,
) -> impl IntoResponse {
    for backend in &state.backends {
        if let Err(e) = backend.play_pause().await {
            tracing::warn!(error = %e, "play_pause failed");
        }
    }
    "ok"
}

async fn next_handler(
    axum::extract::State(state): axum::extract::State<Arc<State>>,
) -> impl IntoResponse {
    for backend in &state.backends {
        if let Err(e) = backend.next_track().await {
            tracing::warn!(error = %e, "next_track failed");
        }
    }
    "ok"
}
