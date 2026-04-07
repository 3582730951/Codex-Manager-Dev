mod browser_assist;
mod bus;
mod config;
mod http;
mod models;
mod openai_auth;
mod scheduler;
mod state;
mod storage;
mod upstream;

use axum::Router;
use tokio::net::TcpListener;
use tracing::info;

use crate::{config::Config, state::AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "codex_manager_server=info,axum=info".to_string()),
        )
        .init();

    let config = Config::from_env();
    let state = AppState::new(config.clone()).await;

    let data_router = http::data::router(state.clone());
    let admin_router = http::admin::router(state.clone());

    let data_listener = TcpListener::bind(config.data_addr())
        .await
        .expect("bind data listener");
    let admin_listener = TcpListener::bind(config.admin_addr())
        .await
        .expect("bind admin listener");

    info!("data plane listening on {}", config.data_addr());
    info!("admin plane listening on {}", config.admin_addr());

    let data = serve_router(data_listener, data_router);
    let admin = serve_router(admin_listener, admin_router);

    tokio::select! {
        result = data => {
            if let Err(error) = result {
                tracing::error!(%error, "data plane exited");
            }
        }
        result = admin => {
            if let Err(error) = result {
                tracing::error!(%error, "admin plane exited");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown requested");
        }
    }
}

async fn serve_router(listener: TcpListener, router: Router) -> std::io::Result<()> {
    axum::serve(listener, router).await
}
