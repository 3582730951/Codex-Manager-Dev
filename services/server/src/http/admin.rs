use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use uuid::Uuid;

use crate::{
    browser_assist,
    models::{
        BrowserTaskRequest, CreateGatewayApiKeyRequest, CreateGatewayUserRequest,
        CreateTenantRequest, ImportAccountRequest, OpenAiLoginCompleteRequest,
        OpenAiLoginStartRequest, RouteEventRequest, UpdateGatewayUserRequest,
    },
    state::AppState,
};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/tenants", get(tenants).post(create_tenant))
        .route("/api/v1/accounts", get(accounts))
        .route("/api/v1/accounts/models/refresh", post(refresh_account_models))
        .route("/api/v1/egress-slots", get(egress_slots))
        .route("/api/v1/leases", get(leases))
        .route("/api/v1/cache-metrics", get(cache_metrics))
        .route("/api/v1/cf-incidents", get(cf_incidents))
        .route("/api/v1/accounts/import", post(import_account))
        .route("/api/v1/users", get(users).post(create_user))
        .route("/api/v1/users/{user_id}", put(update_user))
        .route(
            "/api/v1/gateway/api-keys",
            get(api_keys).post(create_api_key),
        )
        .route("/api/v1/browser/tasks", get(browser_tasks))
        .route("/api/v1/browser/tasks/login", post(browser_login))
        .route("/api/v1/browser/tasks/recover", post(browser_recover))
        .route("/api/v1/openai/login/start", post(openai_login_start))
        .route("/api/v1/openai/login/{login_id}", get(openai_login_status))
        .route("/api/v1/openai/login/complete", post(openai_login_complete))
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

async fn refresh_account_models(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::models::AccountSummary>>, (StatusCode, Json<serde_json::Value>)> {
    match state.refresh_account_models().await {
        Ok(accounts) => Ok(Json(accounts)),
        Err(message) => Err((
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "model_refresh_failed"
                }
            })),
        )),
    }
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

async fn users(State(state): State<AppState>) -> Json<Vec<crate::models::GatewayUserView>> {
    Json(state.list_gateway_users().await)
}

async fn create_user(
    State(state): State<AppState>,
    Json(payload): Json<CreateGatewayUserRequest>,
) -> Result<Json<crate::models::CreatedGatewayUser>, (StatusCode, Json<serde_json::Value>)> {
    match state.create_gateway_user(payload).await {
        Ok(user) => Ok(Json(user)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "create_user_failed"
                }
            })),
        )),
    }
}

async fn update_user(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<UpdateGatewayUserRequest>,
) -> Result<Json<crate::models::GatewayUserView>, (StatusCode, Json<serde_json::Value>)> {
    match state.update_gateway_user(user_id, payload).await {
        Ok(user) => Ok(Json(user)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "update_user_failed"
                }
            })),
        )),
    }
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

async fn openai_login_start(
    State(state): State<AppState>,
    Json(payload): Json<OpenAiLoginStartRequest>,
) -> Result<Json<crate::models::OpenAiLoginStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    match state.start_openai_login(payload).await {
        Ok(response) => Ok(Json(response)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "openai_login_start_failed"
                }
            })),
        )),
    }
}

async fn openai_login_status(
    State(state): State<AppState>,
    Path(login_id): Path<String>,
) -> Result<Json<crate::models::OpenAiLoginSessionView>, (StatusCode, Json<serde_json::Value>)> {
    match state.openai_login_status(&login_id).await {
        Some(session) => Ok(Json(session)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": {
                    "message": "Login session not found.",
                    "type": "not_found"
                }
            })),
        )),
    }
}

async fn openai_login_complete(
    State(state): State<AppState>,
    Json(payload): Json<OpenAiLoginCompleteRequest>,
) -> Result<Json<crate::models::UpstreamAccount>, (StatusCode, Json<serde_json::Value>)> {
    match state.complete_openai_login(payload).await {
        Ok(account) => Ok(Json(account)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "openai_login_complete_failed"
                }
            })),
        )),
    }
}
