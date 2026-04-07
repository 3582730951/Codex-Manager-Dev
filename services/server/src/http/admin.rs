use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::{
    browser_assist,
    models::{
        BrowserTaskRequest, CreateGatewayApiKeyRequest, CreateTenantRequest, ImportAccountRequest,
        RouteEventRequest,
    },
    state::AppState,
};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/tenants", get(tenants).post(create_tenant))
        .route("/api/v1/accounts", get(accounts))
        .route("/api/v1/egress-slots", get(egress_slots))
        .route("/api/v1/leases", get(leases))
        .route("/api/v1/cache-metrics", get(cache_metrics))
        .route("/api/v1/cf-incidents", get(cf_incidents))
        .route("/api/v1/accounts/import", post(import_account))
        .route(
            "/api/v1/gateway/api-keys",
            get(api_keys).post(create_api_key),
        )
        .route("/api/v1/browser/tasks", get(browser_tasks))
        .route("/api/v1/browser/tasks/login", post(browser_login))
        .route("/api/v1/browser/tasks/recover", post(browser_recover))
        .route(
            "/api/v1/accounts/{account_id}/route-events",
            post(route_event),
        )
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "service": "server-admin",
        "status": "ok",
        "storageMode": if state.postgres_connected() { "postgres+memory" } else { "memory-only" },
        "postgresConnected": state.postgres_connected(),
        "redisConnected": state.redis_connected(),
        "redisChannel": state.config.redis_channel,
        "instanceId": state.config.instance_id,
        "postgresUrl": state.config.postgres_url,
        "redisUrl": state.config.redis_url,
        "browserAssistUrl": state.config.browser_assist_url,
        "directProxyConfigured": state.config.direct_proxy_url.is_some(),
        "warpProxyConfigured": state.config.warp_proxy_url.is_some(),
        "browserAssistDirectProxyConfigured": state.config.browser_assist_direct_proxy_url.is_some(),
        "browserAssistWarpProxyConfigured": state.config.browser_assist_warp_proxy_url.is_some()
    }))
}

async fn dashboard(State(state): State<AppState>) -> Json<crate::models::DashboardSnapshot> {
    let mut snapshot = state.dashboard_snapshot().await;
    let tasks = browser_assist::list_tasks(&state.config.browser_assist_url).await;
    snapshot.counts.browser_tasks = tasks.len();
    snapshot.browser_tasks = tasks;
    Json(snapshot)
}

async fn tenants(State(state): State<AppState>) -> Json<Vec<crate::models::Tenant>> {
    Json(state.list_tenants().await)
}

async fn accounts(State(state): State<AppState>) -> Json<Vec<crate::models::AccountSummary>> {
    Json(state.list_accounts().await)
}

async fn egress_slots(State(state): State<AppState>) -> Json<Vec<crate::models::EgressSlot>> {
    Json(state.list_egress_slots().await)
}

async fn leases(State(state): State<AppState>) -> Json<Vec<crate::models::CliLease>> {
    Json(state.list_leases().await)
}

async fn cache_metrics(State(state): State<AppState>) -> Json<crate::models::CacheMetrics> {
    Json(state.cache_metrics().await)
}

async fn cf_incidents(State(state): State<AppState>) -> Json<Vec<crate::models::CfIncident>> {
    Json(state.list_cf_incidents().await)
}

async fn create_tenant(
    State(state): State<AppState>,
    Json(payload): Json<CreateTenantRequest>,
) -> Json<crate::models::Tenant> {
    Json(state.create_tenant(payload).await)
}

async fn import_account(
    State(state): State<AppState>,
    Json(payload): Json<ImportAccountRequest>,
) -> Json<crate::models::UpstreamAccount> {
    Json(state.import_account(payload).await)
}

async fn api_keys(State(state): State<AppState>) -> Json<Vec<crate::models::GatewayApiKeyView>> {
    Json(state.list_api_keys().await)
}

async fn create_api_key(
    State(state): State<AppState>,
    Json(payload): Json<CreateGatewayApiKeyRequest>,
) -> Result<Json<crate::models::CreatedGatewayApiKey>, (StatusCode, Json<serde_json::Value>)> {
    let Some(api_key) = state.create_api_key(payload).await else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": {
                    "message": "Tenant not found.",
                    "type": "not_found"
                }
            })),
        ));
    };
    Ok(Json(api_key))
}

async fn route_event(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
    Json(payload): Json<RouteEventRequest>,
) -> Json<serde_json::Value> {
    let state = state.record_route_event(&account_id, payload).await;
    Json(serde_json::json!({
        "state": state
    }))
}

async fn browser_tasks(State(state): State<AppState>) -> Json<Vec<crate::models::BrowserTask>> {
    Json(browser_assist::list_tasks(&state.config.browser_assist_url).await)
}

async fn browser_login(
    State(state): State<AppState>,
    Json(payload): Json<BrowserTaskRequest>,
) -> Result<Json<crate::models::BrowserTask>, (StatusCode, Json<serde_json::Value>)> {
    match browser_assist::submit_task(
        &state.config.browser_assist_url,
        "login",
        browser_assist::BrowserTaskPayload {
            account_id: payload.account_id,
            notes: payload.notes,
            login_url: payload.login_url,
            headless: payload.headless,
            provider: payload.provider,
            email: payload.email,
            password: payload.password,
            otp_code: payload.otp_code,
            route_mode: payload.route_mode,
        },
    )
    .await
    {
        Ok(task) => Ok(Json(task)),
        Err(status) => Err((
            status,
            Json(serde_json::json!({
                "error": {
                    "message": "Browser assist unavailable.",
                    "type": "browser_assist_unavailable"
                }
            })),
        )),
    }
}

async fn browser_recover(
    State(state): State<AppState>,
    Json(payload): Json<BrowserTaskRequest>,
) -> Result<Json<crate::models::BrowserTask>, (StatusCode, Json<serde_json::Value>)> {
    match browser_assist::submit_task(
        &state.config.browser_assist_url,
        "recover",
        browser_assist::BrowserTaskPayload {
            account_id: payload.account_id,
            notes: payload.notes,
            login_url: payload.login_url,
            headless: payload.headless,
            provider: payload.provider,
            email: payload.email,
            password: payload.password,
            otp_code: payload.otp_code,
            route_mode: payload.route_mode,
        },
    )
    .await
    {
        Ok(task) => Ok(Json(task)),
        Err(status) => Err((
            status,
            Json(serde_json::json!({
                "error": {
                    "message": "Browser assist unavailable.",
                    "type": "browser_assist_unavailable"
                }
            })),
        )),
    }
}
