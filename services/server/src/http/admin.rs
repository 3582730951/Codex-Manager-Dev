use axum::{
    Json, Router,
    body::Body,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    browser_assist,
    models::{
        BrowserTaskRequest, CodexAppSessionRequest, CompactConversationThreadRequest,
        CreateGatewayApiKeyRequest, CreateGatewayUserRequest, CreateTenantRequest,
        ForkConversationThreadRequest, ImportAccountRequest, ManagedRateLimitSnapshot,
        OpenAiLoginCompleteRequest, OpenAiLoginStartRequest, RouteEventRequest,
        StartConversationThreadRequest, UpdateGatewayUserRequest,
    },
    state::{AppState, CodexAppSessionState, GatewayAuthContext},
};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/dashboard/live", get(dashboard_live))
        .route("/api/v1/tenants", get(tenants).post(create_tenant))
        .route("/api/v1/accounts", get(accounts))
        .route(
            "/api/v1/accounts/models/refresh",
            post(refresh_account_models),
        )
        .route(
            "/api/v1/accounts/cleanup/banned",
            post(cleanup_banned_accounts),
        )
        .route(
            "/api/v1/accounts/{account_id}/quota/refresh",
            post(refresh_account_quota),
        )
        .route("/api/v1/accounts/{account_id}", delete(delete_account))
        .route("/api/v1/egress-slots", get(egress_slots))
        .route("/api/v1/leases", get(leases))
        .route("/api/v1/cache-metrics", get(cache_metrics))
        .route("/api/v1/cf-incidents", get(cf_incidents))
        .route("/api/v1/accounts/import", post(import_account))
        .route("/api/internal/threads", get(threads))
        .route("/api/internal/threads/start", post(start_thread))
        .route("/api/internal/threads/fork", post(fork_thread))
        .route("/api/internal/threads/compact", post(compact_thread))
        .route("/api/internal/threads/{thread_id}", get(thread_view))
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
        .route("/api/v1/codex/app-session", post(codex_app_session))
        .route("/api/v1/codex/app-server/ws", get(codex_app_server_ws))
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

async fn dashboard_live(
    State(state): State<AppState>,
) -> Json<crate::models::DashboardLiveSnapshot> {
    Json(state.dashboard_live_snapshot().await)
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

async fn refresh_account_quota(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> Result<Json<crate::models::AccountSummary>, (StatusCode, Json<serde_json::Value>)> {
    match state.refresh_account_quota(&account_id).await {
        Ok(account) => Ok(Json(account)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "quota_refresh_failed"
                }
            })),
        )),
    }
}

async fn delete_account(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    match state.delete_account(&account_id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "account_delete_failed"
                }
            })),
        )),
    }
}

async fn cleanup_banned_accounts(
    State(state): State<AppState>,
) -> Result<Json<crate::models::AccountCleanupResult>, (StatusCode, Json<serde_json::Value>)> {
    match state.cleanup_banned_accounts().await {
        Ok(result) => Ok(Json(result)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "account_cleanup_failed"
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

async fn threads(State(state): State<AppState>) -> Json<Vec<crate::models::ConversationThread>> {
    Json(state.list_conversation_threads().await)
}

async fn start_thread(
    State(state): State<AppState>,
    Json(payload): Json<StartConversationThreadRequest>,
) -> Result<Json<crate::models::ConversationThreadView>, (StatusCode, Json<serde_json::Value>)> {
    match state.start_conversation_thread(payload).await {
        Ok(thread) => Ok(Json(thread)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "thread_start_failed"
                }
            })),
        )),
    }
}

async fn fork_thread(
    State(state): State<AppState>,
    Json(payload): Json<ForkConversationThreadRequest>,
) -> Result<Json<crate::models::ConversationThreadView>, (StatusCode, Json<serde_json::Value>)> {
    match state.fork_conversation_thread(payload).await {
        Ok(thread) => Ok(Json(thread)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "thread_fork_failed"
                }
            })),
        )),
    }
}

async fn compact_thread(
    State(state): State<AppState>,
    Json(payload): Json<CompactConversationThreadRequest>,
) -> Result<Json<crate::models::ConversationThreadView>, (StatusCode, Json<serde_json::Value>)> {
    match state.compact_conversation_thread(payload).await {
        Ok(thread) => Ok(Json(thread)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": message,
                    "type": "thread_compact_failed"
                }
            })),
        )),
    }
}

async fn thread_view(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
) -> Result<Json<crate::models::ConversationThreadView>, (StatusCode, Json<serde_json::Value>)> {
    match state.conversation_thread_view(&thread_id).await {
        Some(thread) => Ok(Json(thread)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": {
                    "message": "Thread not found.",
                    "type": "not_found"
                }
            })),
        )),
    }
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

async fn codex_app_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CodexAppSessionRequest>,
) -> Result<Json<crate::models::CodexAppSessionResponse>, (StatusCode, Json<Value>)> {
    let Some(auth) = gateway_auth_from_headers(&state, &headers).await else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "缺少有效的 Gateway API Key。",
                    "type": "unauthorized"
                }
            })),
        ));
    };
    let websocket_url = websocket_public_url(&headers).map_err(|message| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": message,
                    "type": "codex_app_session_failed"
                }
            })),
        )
    })?;
    match state
        .create_codex_app_session(payload, auth.tenant.id, websocket_url)
        .await
    {
        Ok(session) => Ok(Json(session)),
        Err(message) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": message,
                    "type": "codex_app_session_failed"
                }
            })),
        )),
    }
}

async fn codex_app_server_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    let Some(session) = codex_app_session_from_headers(&state, &headers).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "Unauthorized codex app-server session.",
                    "type": "unauthorized"
                }
            })),
        )
            .into_response();
    };
    ws.on_upgrade(move |socket| async move {
        handle_codex_app_server_socket(socket, state, session).await;
    })
    .into_response()
}

async fn handle_codex_app_server_socket(
    mut socket: WebSocket,
    state: AppState,
    session: CodexAppSessionState,
) {
    let mut initialize_seen = false;
    let mut notifications_ready = false;
    let mut opt_out_notifications = std::collections::HashSet::<String>::new();

    loop {
        let Some(message) = socket.recv().await else {
            break;
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(%error, "codex app-server websocket receive failed");
                break;
            }
        };

        match message {
            Message::Text(text) => {
                if handle_codex_app_server_text(
                    &mut socket,
                    &state,
                    &session,
                    &mut initialize_seen,
                    &mut notifications_ready,
                    &mut opt_out_notifications,
                    text.as_str(),
                )
                .await
                .is_err()
                {
                    break;
                }
            }
            Message::Binary(_) => {
                if send_app_server_error(
                    &mut socket,
                    Value::Null,
                    -32600,
                    "binary websocket frames are not supported",
                )
                .await
                .is_err()
                {
                    break;
                }
            }
            Message::Ping(payload) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Message::Pong(_) => {}
            Message::Close(_) => break,
        }
    }
}

async fn handle_codex_app_server_text(
    socket: &mut WebSocket,
    state: &AppState,
    session: &CodexAppSessionState,
    initialize_seen: &mut bool,
    notifications_ready: &mut bool,
    opt_out_notifications: &mut std::collections::HashSet<String>,
    message: &str,
) -> Result<(), ()> {
    let value = match serde_json::from_str::<Value>(message) {
        Ok(value) => value,
        Err(error) => {
            send_app_server_error(
                socket,
                Value::Null,
                -32700,
                &format!("invalid JSON: {error}"),
            )
            .await?;
            return Ok(());
        }
    };
    let Some(object) = value.as_object() else {
        send_app_server_error(socket, Value::Null, -32600, "invalid request payload").await?;
        return Ok(());
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        send_app_server_error(
            socket,
            id.clone().unwrap_or(Value::Null),
            -32600,
            "invalid request payload: missing method",
        )
        .await?;
        return Ok(());
    };
    let method = method.to_string();

    if id.is_none() {
        match method.as_str() {
            "initialized" => {
                if !*initialize_seen {
                    return Ok(());
                }
                *notifications_ready = true;
                emit_account_notifications(socket, state, session, opt_out_notifications).await?;
            }
            _ => {}
        }
        return Ok(());
    }

    let id = id.unwrap_or(Value::Null);
    if !*initialize_seen && method != "initialize" {
        send_app_server_error(socket, id, -32002, "Not initialized").await?;
        return Ok(());
    }

    match method.as_str() {
        "initialize" => {
            if *initialize_seen {
                send_app_server_error(socket, id, -32003, "Already initialized").await?;
                return Ok(());
            }
            opt_out_notifications.clear();
            opt_out_notifications.extend(
                object
                    .get("params")
                    .and_then(|params| params.get("capabilities"))
                    .and_then(|capabilities| capabilities.get("optOutNotificationMethods"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            );
            *initialize_seen = true;
            send_app_server_result(socket, id, initialize_response_value()).await?;
        }
        "account/read" => {
            let refresh_token = object
                .get("params")
                .and_then(|params| params.get("refreshToken"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if refresh_token {
                if let Err(error) = state
                    .refresh_managed_account(&session.account_id, true)
                    .await
                {
                    tracing::warn!(
                        account_id = %session.account_id,
                        %error,
                        "codex app-server account/read refresh failed"
                    );
                }
            }
            let result = account_read_result(state, session).await;
            send_app_server_result(socket, id, result).await?;
            if *notifications_ready {
                emit_account_notifications(socket, state, session, opt_out_notifications).await?;
            }
        }
        "account/rateLimits/read" => {
            if let Err(error) = state
                .refresh_managed_account(&session.account_id, false)
                .await
            {
                tracing::warn!(
                    account_id = %session.account_id,
                    %error,
                    "codex app-server rateLimits refresh failed"
                );
            }
            let result = rate_limits_result(state, session).await;
            send_app_server_result(socket, id, result).await?;
            if *notifications_ready {
                emit_rate_limit_notification(socket, state, session, opt_out_notifications).await?;
            }
        }
        "model/list" => {
            let result = model_list_result(state, session, object.get("params")).await;
            send_app_server_result(socket, id, result).await?;
        }
        "getAuthStatus" => {
            let result = auth_status_result(state, session).await;
            send_app_server_result(socket, id, result).await?;
        }
        "mcpServerStatus/list" | "mcpServers/list" => {
            let result = mcp_server_status_list_result(object.get("params"));
            send_app_server_result(socket, id, result).await?;
        }
        _ => {
            send_app_server_error(socket, id, -32601, "Method not found").await?;
        }
    }

    Ok(())
}

async fn emit_account_notifications(
    socket: &mut WebSocket,
    state: &AppState,
    session: &CodexAppSessionState,
    opt_out_notifications: &std::collections::HashSet<String>,
) -> Result<(), ()> {
    if !opt_out_notifications.contains("account/updated") {
        let payload = account_updated_notification(state, session).await;
        send_app_server_notification(socket, "account/updated", payload).await?;
    }
    emit_rate_limit_notification(socket, state, session, opt_out_notifications).await
}

async fn emit_rate_limit_notification(
    socket: &mut WebSocket,
    state: &AppState,
    session: &CodexAppSessionState,
    opt_out_notifications: &std::collections::HashSet<String>,
) -> Result<(), ()> {
    if opt_out_notifications.contains("account/rateLimits/updated") {
        return Ok(());
    }
    let payload = json!({
        "rateLimits": current_rate_limits_snapshot(state, session).await,
    });
    send_app_server_notification(socket, "account/rateLimits/updated", payload).await
}

async fn account_read_result(state: &AppState, session: &CodexAppSessionState) -> Value {
    let summary = managed_account_summary(state, session).await;
    json!({
        "account": summary.as_ref().and_then(account_value_from_summary),
        "workspaceRole": summary
            .as_ref()
            .and_then(|summary| sanitize_workspace_role(summary.workspace_role.as_deref())),
        "isWorkspaceOwner": summary.as_ref().and_then(|summary| summary.is_workspace_owner),
        "requiresOpenaiAuth": requires_openai_auth(summary.as_ref())
    })
}

async fn account_updated_notification(state: &AppState, session: &CodexAppSessionState) -> Value {
    let summary = managed_account_summary(state, session).await;
    json!({
        "authMode": summary
            .as_ref()
            .and_then(|summary| summary.auth_mode.clone()),
        "planType": summary
            .as_ref()
            .and_then(|summary| sanitize_plan_type(summary.plan_type.as_deref())),
        "workspaceRole": summary
            .as_ref()
            .and_then(|summary| sanitize_workspace_role(summary.workspace_role.as_deref())),
        "isWorkspaceOwner": summary.as_ref().and_then(|summary| summary.is_workspace_owner),
    })
}

async fn auth_status_result(state: &AppState, session: &CodexAppSessionState) -> Value {
    let summary = managed_account_summary(state, session).await;
    let auth_method = if let Some(summary) = summary.as_ref() {
        summary.auth_mode.clone()
    } else {
        state
            .app_server_auth_mode_for_account(&session.account_id)
            .await
    };
    json!({
        "authMethod": auth_method,
        "authToken": Value::Null,
        "requiresOpenaiAuth": requires_openai_auth(summary.as_ref()),
    })
}

async fn rate_limits_result(state: &AppState, session: &CodexAppSessionState) -> Value {
    let summary = managed_account_summary(state, session).await;
    let rate_limits = summary
        .as_ref()
        .and_then(|summary| summary.rate_limits.clone())
        .unwrap_or_default();
    let rate_limits_by_limit_id = summary
        .as_ref()
        .map(|summary| {
            if summary.rate_limits_by_limit_id.is_empty() {
                Value::Null
            } else {
                json!(summary.rate_limits_by_limit_id)
            }
        })
        .unwrap_or(Value::Null);

    json!({
        "rateLimits": rate_limits,
        "rateLimitsByLimitId": rate_limits_by_limit_id,
    })
}

async fn current_rate_limits_snapshot(
    state: &AppState,
    session: &CodexAppSessionState,
) -> ManagedRateLimitSnapshot {
    managed_account_summary(state, session)
        .await
        .and_then(|summary| summary.rate_limits.clone())
        .unwrap_or_default()
}

async fn managed_account_summary(
    state: &AppState,
    session: &CodexAppSessionState,
) -> Option<crate::models::AccountSummary> {
    state
        .list_accounts()
        .await
        .into_iter()
        .find(|account| account.id == session.account_id && account.tenant_id == session.tenant_id)
}

async fn model_list_result(
    state: &AppState,
    session: &CodexAppSessionState,
    params: Option<&Value>,
) -> Value {
    let models = managed_account_summary(state, session)
        .await
        .map(|summary| normalize_account_models(&summary.models))
        .filter(|models| !models.is_empty())
        .unwrap_or_else(default_app_server_models);
    let include_hidden = params
        .and_then(|params| params.get("includeHidden"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let limit = params
        .and_then(|params| params.get("limit"))
        .and_then(Value::as_u64)
        .and_then(|limit| usize::try_from(limit).ok());
    let cursor = params
        .and_then(|params| params.get("cursor"))
        .and_then(Value::as_str);

    build_model_list_response(&models, include_hidden, limit, cursor)
}

fn build_model_list_response(
    models: &[String],
    include_hidden: bool,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Value {
    let default_model = preferred_default_model(models).map(str::to_string);
    let filtered = models
        .iter()
        .map(|model| app_server_model_metadata(model, default_model.as_deref() == Some(model)))
        .filter(|model| include_hidden || !model.hidden)
        .collect::<Vec<_>>();
    let start = cursor
        .and_then(parse_model_list_cursor)
        .unwrap_or_default()
        .min(filtered.len());
    let end = limit
        .map(|limit| start.saturating_add(limit).min(filtered.len()))
        .unwrap_or(filtered.len());
    let next_cursor = (end < filtered.len()).then(|| end.to_string());

    json!({
        "data": filtered[start..end].to_vec(),
        "nextCursor": next_cursor,
    })
}

fn parse_model_list_cursor(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<usize>().ok()
}

fn mcp_server_status_list_result(params: Option<&Value>) -> Value {
    let limit = params
        .and_then(|params| params.get("limit"))
        .and_then(Value::as_u64)
        .and_then(|limit| usize::try_from(limit).ok());
    let cursor = params
        .and_then(|params| params.get("cursor"))
        .and_then(Value::as_str);
    let all_servers = Vec::<Value>::new();
    let start = cursor
        .and_then(parse_model_list_cursor)
        .unwrap_or_default()
        .min(all_servers.len());
    let end = limit
        .map(|limit| start.saturating_add(limit).min(all_servers.len()))
        .unwrap_or(all_servers.len());
    let next_cursor = (end < all_servers.len()).then(|| end.to_string());

    json!({
        "data": all_servers[start..end].to_vec(),
        "nextCursor": next_cursor,
    })
}

fn preferred_default_model(models: &[String]) -> Option<&str> {
    for preferred in ["gpt-5.4", "gpt-5.3-codex", "gpt-5.2"] {
        if models.iter().any(|model| model == preferred) {
            return Some(preferred);
        }
    }
    models.first().map(String::as_str)
}

fn normalize_account_models(models: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() || normalized.iter().any(|existing| existing == model) {
            continue;
        }
        normalized.push(model.to_string());
    }
    normalized
}

fn default_app_server_models() -> Vec<String> {
    vec![
        "gpt-5.4".to_string(),
        "gpt-5.3-codex".to_string(),
        "gpt-5.2".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppServerModelMetadata {
    id: String,
    model: String,
    upgrade: Option<String>,
    upgrade_info: Option<AppServerModelUpgradeInfo>,
    availability_nux: Option<AppServerModelAvailabilityNux>,
    display_name: String,
    description: String,
    hidden: bool,
    supported_reasoning_efforts: Vec<AppServerReasoningEffortOption>,
    default_reasoning_effort: String,
    input_modalities: Vec<String>,
    supports_personality: bool,
    additional_speed_tiers: Vec<String>,
    is_default: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppServerModelUpgradeInfo {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    upgrade_copy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    migration_markdown: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppServerModelAvailabilityNux {
    message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppServerReasoningEffortOption {
    reasoning_effort: String,
    description: String,
}

fn app_server_model_metadata(model: &str, is_default: bool) -> AppServerModelMetadata {
    let known = known_model_metadata(model);
    AppServerModelMetadata {
        id: model.to_string(),
        model: model.to_string(),
        upgrade: known.upgrade.as_ref().map(|upgrade| upgrade.model.clone()),
        upgrade_info: known.upgrade,
        availability_nux: known.availability_nux,
        display_name: known.display_name,
        description: known.description,
        hidden: known.hidden,
        supported_reasoning_efforts: known.supported_reasoning_efforts,
        default_reasoning_effort: known.default_reasoning_effort,
        input_modalities: known.input_modalities,
        supports_personality: known.supports_personality,
        additional_speed_tiers: known.additional_speed_tiers,
        is_default,
    }
}

struct KnownModelMetadata {
    display_name: String,
    description: String,
    hidden: bool,
    supported_reasoning_efforts: Vec<AppServerReasoningEffortOption>,
    default_reasoning_effort: String,
    input_modalities: Vec<String>,
    supports_personality: bool,
    additional_speed_tiers: Vec<String>,
    availability_nux: Option<AppServerModelAvailabilityNux>,
    upgrade: Option<AppServerModelUpgradeInfo>,
}

fn known_model_metadata(model: &str) -> KnownModelMetadata {
    match model {
        "gpt-5.4" => KnownModelMetadata {
            display_name: "gpt-5.4".to_string(),
            description: "Latest frontier agentic coding model.".to_string(),
            hidden: false,
            supported_reasoning_efforts: standard_reasoning_efforts(
                "Fast responses with lighter reasoning",
                "Balances speed and reasoning depth for everyday tasks",
                "Greater reasoning depth for complex problems",
                "Extra high reasoning depth for complex problems",
            ),
            default_reasoning_effort: "medium".to_string(),
            input_modalities: vec!["text".to_string(), "image".to_string()],
            supports_personality: false,
            additional_speed_tiers: vec!["fast".to_string()],
            availability_nux: None,
            upgrade: None,
        },
        "gpt-5.3-codex" => KnownModelMetadata {
            display_name: "gpt-5.3-codex".to_string(),
            description: "Latest frontier agentic coding model.".to_string(),
            hidden: false,
            supported_reasoning_efforts: standard_reasoning_efforts(
                "Fast responses with lighter reasoning",
                "Balances speed and reasoning depth for everyday tasks",
                "Greater reasoning depth for complex problems",
                "Extra high reasoning depth for complex problems",
            ),
            default_reasoning_effort: "medium".to_string(),
            input_modalities: vec!["text".to_string(), "image".to_string()],
            supports_personality: false,
            additional_speed_tiers: Vec::new(),
            availability_nux: None,
            upgrade: Some(AppServerModelUpgradeInfo {
                model: "gpt-5.4".to_string(),
                upgrade_copy: None,
                model_link: None,
                migration_markdown: None,
            }),
        },
        "gpt-5.2" => KnownModelMetadata {
            display_name: "gpt-5.2".to_string(),
            description:
                "Latest frontier model with improvements across knowledge, reasoning and coding"
                    .to_string(),
            hidden: false,
            supported_reasoning_efforts: standard_reasoning_efforts(
                "Balances speed with some reasoning; useful for straightforward queries and short explanations",
                "Provides a solid balance of reasoning depth and latency for general-purpose tasks",
                "Maximizes reasoning depth for complex or ambiguous problems",
                "Extra high reasoning for complex problems",
            ),
            default_reasoning_effort: "medium".to_string(),
            input_modalities: vec!["text".to_string(), "image".to_string()],
            supports_personality: false,
            additional_speed_tiers: Vec::new(),
            availability_nux: None,
            upgrade: Some(AppServerModelUpgradeInfo {
                model: "gpt-5.4".to_string(),
                upgrade_copy: None,
                model_link: None,
                migration_markdown: None,
            }),
        },
        _ => KnownModelMetadata {
            display_name: model.to_string(),
            description: "Available model from the managed account.".to_string(),
            hidden: false,
            supported_reasoning_efforts: standard_reasoning_efforts(
                "Fast responses with lighter reasoning",
                "Balanced reasoning for everyday tasks",
                "Greater reasoning depth for complex problems",
                "Extra high reasoning depth for complex problems",
            ),
            default_reasoning_effort: "medium".to_string(),
            input_modalities: vec!["text".to_string(), "image".to_string()],
            supports_personality: false,
            additional_speed_tiers: Vec::new(),
            availability_nux: None,
            upgrade: None,
        },
    }
}

fn standard_reasoning_efforts(
    low: &str,
    medium: &str,
    high: &str,
    xhigh: &str,
) -> Vec<AppServerReasoningEffortOption> {
    vec![
        AppServerReasoningEffortOption {
            reasoning_effort: "low".to_string(),
            description: low.to_string(),
        },
        AppServerReasoningEffortOption {
            reasoning_effort: "medium".to_string(),
            description: medium.to_string(),
        },
        AppServerReasoningEffortOption {
            reasoning_effort: "high".to_string(),
            description: high.to_string(),
        },
        AppServerReasoningEffortOption {
            reasoning_effort: "xhigh".to_string(),
            description: xhigh.to_string(),
        },
    ]
}

fn account_value_from_summary(summary: &crate::models::AccountSummary) -> Option<Value> {
    match summary.auth_mode.as_deref() {
        Some("apikey") => Some(json!({
            "type": "apiKey",
        })),
        Some("chatgpt") => {
            let email = summary.chatgpt_email.as_deref()?;
            Some(json!({
                "type": "chatgpt",
                "email": email,
                "planType": sanitize_plan_type(summary.plan_type.as_deref())
                    .unwrap_or_else(|| "unknown".to_string()),
            }))
        }
        _ => None,
    }
}

fn requires_openai_auth(summary: Option<&crate::models::AccountSummary>) -> bool {
    summary
        .and_then(|summary| summary.base_url.as_deref())
        .map(|base_url| !base_url.trim().is_empty())
        .unwrap_or(true)
}

fn sanitize_plan_type(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "free"
        | "go"
        | "plus"
        | "pro"
        | "team"
        | "self_serve_business_usage_based"
        | "business"
        | "enterprise_cbp_usage_based"
        | "enterprise"
        | "edu"
        | "unknown" => Some(normalized),
        _ => Some("unknown".to_string()),
    }
}

fn sanitize_workspace_role(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "account-owner" | "account-admin" | "standard-user" => Some(normalized),
        _ => None,
    }
}

fn initialize_response_value() -> Value {
    let codex_home = std::env::var("CODEX_HOME")
        .ok()
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.join(".codex").to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "/tmp/.codex".to_string());
    json!({
        "userAgent": format!("codex-manager-app-server/{}", env!("CARGO_PKG_VERSION")),
        "codexHome": codex_home,
        "platformFamily": std::env::consts::FAMILY,
        "platformOs": std::env::consts::OS,
    })
}

async fn send_app_server_result(
    socket: &mut WebSocket,
    id: Value,
    result: Value,
) -> Result<(), ()> {
    send_app_server_json(
        socket,
        &json!({
            "id": id,
            "result": result,
        }),
    )
    .await
}

async fn send_app_server_error(
    socket: &mut WebSocket,
    id: Value,
    code: i64,
    message: &str,
) -> Result<(), ()> {
    send_app_server_json(
        socket,
        &json!({
            "id": id,
            "error": {
                "code": code,
                "message": message,
            }
        }),
    )
    .await
}

async fn send_app_server_notification(
    socket: &mut WebSocket,
    method: &str,
    params: Value,
) -> Result<(), ()> {
    send_app_server_json(
        socket,
        &json!({
            "method": method,
            "params": params,
        }),
    )
    .await
}

async fn send_app_server_json(socket: &mut WebSocket, value: &Value) -> Result<(), ()> {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
}

async fn codex_app_session_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<CodexAppSessionState> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    let session = state.codex_app_session_for_token(token).await?;
    if session.expires_at <= Utc::now() {
        return None;
    }
    Some(session)
}

async fn gateway_auth_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<GatewayAuthContext> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    state.auth_context_for_bearer(token).await
}

fn websocket_public_url(headers: &HeaderMap) -> Result<String, String> {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(axum::http::header::HOST))
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "缺少 Host 头，无法生成 websocket 地址。".to_string())?;
    let forwarded_proto = headers
        .get("x-forwarded-proto")
        .or_else(|| headers.get("x-forwarded-scheme"))
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    let scheme = if forwarded_proto.eq_ignore_ascii_case("https")
        || forwarded_proto.eq_ignore_ascii_case("wss")
    {
        "wss"
    } else {
        "ws"
    };
    Ok(format!("{scheme}://{host}/api/v1/codex/app-server/ws"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AccountAvailabilityState, AccountSummary, ManagedAccountStatus, RouteMode,
    };
    use uuid::Uuid;

    fn sample_account_summary() -> AccountSummary {
        AccountSummary {
            id: "acc_test".to_string(),
            tenant_id: Uuid::nil(),
            label: "Test".to_string(),
            models: vec!["gpt-5.4".to_string()],
            current_mode: RouteMode::Direct,
            route_mode: RouteMode::Direct,
            cooldown_level: 0,
            cooldown_until: None,
            direct_cf_streak: 0,
            warp_cf_streak: 0,
            success_streak: 0,
            quota_headroom: 1.0,
            quota_headroom_5h: 1.0,
            quota_headroom_7d: 1.0,
            near_quota_guard_enabled: false,
            health_score: 0.9,
            egress_stability: 0.9,
            inflight: 0,
            capacity: 4,
            has_credential: true,
            base_url: Some("https://api.openai.com/v1".to_string()),
            chatgpt_account_id: None,
            auth_mode: Some("apikey".to_string()),
            chatgpt_email: None,
            plan_type: None,
            workspace_role: None,
            is_workspace_owner: None,
            status: ManagedAccountStatus::Active,
            status_reason: None,
            last_error: None,
            rate_limits: None,
            rate_limits_by_limit_id: Default::default(),
            managed_state_refreshed_at: None,
            availability_state: AccountAvailabilityState::Routable,
            availability_reason: None,
            availability_reset_at: None,
            egress_group: "direct-native".to_string(),
            proxy_enabled: false,
        }
    }

    #[test]
    fn preferred_default_model_uses_codex_priority() {
        let models = vec![
            "gpt-5.2".to_string(),
            "gpt-5.4".to_string(),
            "custom-model".to_string(),
        ];

        assert_eq!(preferred_default_model(&models), Some("gpt-5.4"));
    }

    #[test]
    fn build_model_list_response_paginates_with_numeric_cursor() {
        let models = vec![
            "gpt-5.4".to_string(),
            "gpt-5.3-codex".to_string(),
            "gpt-5.2".to_string(),
        ];

        let payload = build_model_list_response(&models, false, Some(1), Some("1"));
        let data = payload
            .get("data")
            .and_then(Value::as_array)
            .expect("data array");

        assert_eq!(data.len(), 1);
        assert_eq!(
            data[0].get("model").and_then(Value::as_str),
            Some("gpt-5.3-codex")
        );
        assert_eq!(payload.get("nextCursor").and_then(Value::as_str), Some("2"));
    }

    #[test]
    fn known_model_metadata_matches_official_default_fields() {
        let payload = build_model_list_response(&["gpt-5.4".to_string()], true, None, None);
        let model = payload
            .get("data")
            .and_then(Value::as_array)
            .and_then(|data| data.first())
            .expect("single model entry");

        assert_eq!(model.get("model").and_then(Value::as_str), Some("gpt-5.4"));
        assert_eq!(
            model.get("defaultReasoningEffort").and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(
            model
                .get("additionalSpeedTiers")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn mcp_server_status_list_result_returns_empty_remote_inventory() {
        let payload = mcp_server_status_list_result(Some(&json!({"limit": 10, "cursor": "0"})));

        assert_eq!(
            payload.get("data").and_then(Value::as_array).map(Vec::len),
            Some(0)
        );
        assert_eq!(payload.get("nextCursor"), Some(&Value::Null));
    }

    #[test]
    fn account_value_from_summary_returns_api_key_variant_for_api_key_accounts() {
        let summary = sample_account_summary();
        let account = account_value_from_summary(&summary).expect("account payload");

        assert_eq!(account.get("type").and_then(Value::as_str), Some("apiKey"));
    }

    #[test]
    fn account_value_from_summary_requires_email_for_chatgpt_variant() {
        let mut summary = sample_account_summary();
        summary.auth_mode = Some("chatgptAuthTokens".to_string());

        assert_eq!(account_value_from_summary(&summary), None);
        assert!(requires_openai_auth(Some(&summary)));
    }
}
