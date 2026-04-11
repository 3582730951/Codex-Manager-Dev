use std::{collections::BTreeMap, convert::Infallible, pin::Pin, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        DefaultBodyLimit, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, Response, StatusCode, header::CONTENT_TYPE},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    models::{
        ChatCompletionsRequest, ChatMessage, CliLease, GatewayApiKey, LeaseSelectionRequest,
        RequestLogEntry, RequestLogUsage, ResponsesRequest, RouteEventRequest, UpstreamCredential,
    },
    openai_auth::DEFAULT_ORIGINATOR,
    state::{
        AppState, GatewayAuthContext, LeaseSelectionExhausted, LeaseSelectionExhaustedKind,
        LeaseSelectionOutcome, ReplayPlan,
    },
    upstream::{
        ForwardContext, UpstreamFailure, UpstreamFailureKind, UpstreamFailureSubkind,
        classify_failure_body,
    },
};

#[derive(Debug)]
struct ForwardSuccess {
    response: Response<Body>,
    output_summary: Option<String>,
    usage: RequestLogUsage,
    observed_model: Option<String>,
    response_id: Option<String>,
    response_output_items: Vec<Value>,
}

impl Default for ForwardSuccess {
    fn default() -> Self {
        Self {
            response: Response::new(Body::empty()),
            output_summary: None,
            usage: RequestLogUsage::default(),
            observed_model: None,
            response_id: None,
            response_output_items: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct RequestLogSeed {
    api_key: GatewayApiKey,
    tenant_id: Uuid,
    principal_id: String,
    endpoint: &'static str,
    method: &'static str,
    requested_model: String,
    effective_model: String,
    reasoning_effort: Option<String>,
}

#[derive(Clone, Debug)]
struct RequestRoutingScope {
    principal_id: String,
    lease_principal_id: String,
    placement_affinity_key: String,
    session_key: String,
    window_id: Option<String>,
    parent_thread_id: Option<String>,
    thread_family_id: Option<String>,
    continuity_mode: ContinuityMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContinuityMode {
    CodexWindow,
    SessionAffinity,
    EphemeralRequest,
}

impl ContinuityMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::CodexWindow => "codex_window",
            Self::SessionAffinity => "session_affinity",
            Self::EphemeralRequest => "ephemeral_request",
        }
    }
}

#[derive(Clone, Debug)]
struct ContinuityError {
    message: String,
}

enum ForwardOutcome {
    Response(ForwardSuccess),
    HiddenFailure(UpstreamFailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GatewayFailureReason {
    Queue,
    Quota,
    Capability,
    UpstreamFailure,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/responses", get(responses_ws).post(responses))
        .route("/v1/responses/compact", post(responses_compact))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(DefaultBodyLimit::max(
            state.config.max_data_plane_body_bytes,
        ))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({
        "service": "server-data",
        "status": "ok"
    }))
}

async fn models(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    match authenticated_context(&state, &headers).await {
        Some(context) => {
            let accounts = state.runtime.accounts.read().await;
            let mut items = accounts
                .values()
                .filter(|account| account.tenant_id == context.tenant.id)
                .flat_map(|account| account.models.clone())
                .collect::<Vec<_>>();
            items.sort();
            items.dedup();
            Json(json!({
                "object": "list",
                "data": items.into_iter().map(|id| json!({"id": id, "object": "model"})).collect::<Vec<_>>()
            }))
            .into_response()
        }
        None => unauthorized().into_response(),
    }
}

fn invalid_request(message: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": {
                "message": message,
                "type": "invalid_request_error"
            }
        })),
    )
}

async fn responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ResponsesRequest>,
) -> Response<Body> {
    let Some(auth) = authenticated_context(&state, &headers).await else {
        return unauthorized().into_response();
    };
    let subagent_count = headers.get("x-openai-subagent").map(|_| 1_u32).unwrap_or(0);
    let requested_model = payload.model.clone();
    let effective_model = resolve_effective_model(&auth.api_key, &requested_model);
    log_model_resolution(&auth.api_key, &requested_model, &effective_model);
    let effective_reasoning_effort =
        resolve_effective_reasoning_for_responses(&auth.api_key, &payload);
    let mut payload = apply_responses_policy(
        &payload,
        &effective_model,
        effective_reasoning_effort.as_deref(),
    );
    let model = payload.model.clone();
    let routing = match resolve_request_scope(&state, &auth, &headers, &model).await {
        Ok(routing) => routing,
        Err(error) => return invalid_request(&error.message).into_response(),
    };
    let principal_id = routing.principal_id.clone();
    let cache_affinity_key =
        responses_cache_affinity_key(auth.tenant.id, cache_continuity_anchor(&routing), &payload);
    let input_summary = summarize_value(&payload.input);
    let request_log = RequestLogSeed {
        api_key: auth.api_key.clone(),
        tenant_id: auth.tenant.id,
        principal_id: principal_id.clone(),
        endpoint: "/v1/responses",
        method: "POST",
        requested_model,
        effective_model: model.clone(),
        reasoning_effort: effective_reasoning_effort.clone(),
    };
    let selection_request = LeaseSelectionRequest {
        tenant_id: auth.tenant.id,
        principal_id: routing.lease_principal_id.clone(),
        model: model.clone(),
        reasoning_effort: effective_reasoning_effort.clone(),
        subagent_count,
        cache_affinity_key: cache_affinity_key.clone(),
        placement_affinity_key: routing.placement_affinity_key.clone(),
    };
    let stream_requested = payload.stream.unwrap_or(false);
    let mut recorded_input = false;
    let context = forward_context(&headers, &routing);
    let mut queued_same_lease: Option<(CliLease, String)> = None;
    let mut continuation_recovery = false;
    let mut transparent_compaction_attempted = false;
    let mut compacted_retry_active = false;
    let mut selection = Some(
        match resolve_http_selection_or_wait(
            &state,
            &selection_request,
            &request_log,
            &routing,
            &context,
            subagent_count,
            stream_requested,
            state.config.heartbeat_seconds,
        )
        .await
        {
            Ok(selection) => selection,
            Err(response) => return response,
        },
    );

    let mut attempt = 0usize;
    let mut non_quota_failovers = 0usize;
    let max_non_quota_failovers = non_quota_retry_budget(&state, &selection_request).await;
    loop {
        let Some((lease, prompt_cache_key)) = queued_same_lease.take().or_else(|| {
            selection
                .take()
                .map(|(lease, prompt_cache_key, _warmup)| (lease, prompt_cache_key))
        }) else {
            return gateway_failure_response(
                stream_requested,
                state.config.heartbeat_seconds,
                GatewayFailureReason::UpstreamFailure,
            );
        };
        let execution_guard = state
            .acquire_execution_guard(auth.tenant.id, &lease.account_id, &model)
            .await;
        if !recorded_input {
            state
                .begin_context_turn(
                    &principal_id,
                    &model,
                    lease.generation,
                    input_summary.clone(),
                    normalize_responses_input(payload.input.clone()),
                )
                .await;
            recorded_input = true;
        }
        let Some(credential) = state.credential_for_account(&lease.account_id).await else {
            let _ = state
                .failover_account(&lease.account_id, "credential-missing", 300, true)
                .await;
            tracing::warn!(
                account_id = %lease.account_id,
                principal_id = %principal_id,
                "selected account missing credential, retrying hidden failover"
            );
            if non_quota_failovers == 0 {
                attempt += 1;
                non_quota_failovers += 1;
                selection = Some(
                    match resolve_http_selection_or_wait(
                        &state,
                        &selection_request,
                        &request_log,
                        &routing,
                        &context,
                        subagent_count,
                        stream_requested,
                        state.config.heartbeat_seconds,
                    )
                    .await
                    {
                        Ok(selection) => selection,
                        Err(response) => return response,
                    },
                );
                continue;
            }
            state.discard_pending_context_turn(&principal_id).await;
            return gateway_failure_response(
                stream_requested,
                state.config.heartbeat_seconds,
                GatewayFailureReason::UpstreamFailure,
            );
        };
        let codex_protocol = is_codex_chatgpt_backend(&credential.base_url);
        let previous_response_id = payload
            .extra
            .get("previous_response_id")
            .and_then(Value::as_str);
        let replay_plan = if continuation_recovery {
            state
                .continuation_recovery_plan_for_request(&principal_id, previous_response_id)
                .await
        } else if compacted_retry_active {
            ReplayPlan::default()
        } else {
            state
                .replay_plan_for_request(&principal_id, lease.generation, previous_response_id)
                .await
        };
        log_request_attempt(
            &request_log,
            &routing,
            &context,
            &lease,
            replay_plan.fallback_text.as_deref(),
            attempt,
        );
        let replayed_tool_calls =
            if codex_protocol && replay_plan.input_items.is_empty() && !compacted_retry_active {
                state
                    .replay_tool_call_items_for(
                        &principal_id,
                        previous_response_id,
                        &responses_input_function_call_output_call_ids(&payload.input),
                    )
                    .await
            } else {
                Vec::new()
            };
        let upstream_value = responses_payload_for_upstream(
            &payload,
            prompt_cache_key.clone(),
            &replay_plan,
            codex_protocol,
            &replayed_tool_calls,
        );
        let upstream_stream = codex_protocol || stream_requested;

        match state
            .upstream
            .post_json(
                &credential,
                "responses",
                &upstream_value,
                &context,
                upstream_stream,
                lease.route_mode,
            )
            .await
        {
            Ok(response) => {
                if stream_requested {
                    let near_quota_guard = state.near_quota_guard_enabled(&lease.account_id).await;
                    match if near_quota_guard {
                        upstream_stream_response(
                            response,
                            state.clone(),
                            lease.clone(),
                            request_log.clone(),
                            principal_id.clone(),
                            &model,
                            state.config.heartbeat_seconds,
                            execution_guard,
                        )
                        .await
                    } else {
                        passthrough_stream_response(
                            response,
                            state.clone(),
                            lease.clone(),
                            request_log.clone(),
                            principal_id.clone(),
                            &model,
                            state.config.heartbeat_seconds,
                            execution_guard,
                        )
                        .await
                    } {
                        ForwardOutcome::Response(success) => return success.response,
                        ForwardOutcome::HiddenFailure(kind) => {
                            if previous_response_id.is_some()
                                && kind == UpstreamFailureKind::Continuation
                                && !continuation_recovery
                            {
                                continuation_recovery = true;
                                compacted_retry_active = false;
                                payload.extra.remove("previous_response_id");
                                queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                                tracing::info!(
                                    account_id = %lease.account_id,
                                    principal_id = %principal_id,
                                    "retrying responses stream on same lease after continuation recovery fallback"
                                );
                                continue;
                            }
                            if !transparent_compaction_attempted
                                && should_transparent_compact_hidden_failure(kind)
                                && let Some(compacted_output) = compact_retry_output_items(
                                    &state,
                                    &credential,
                                    &lease,
                                    &context,
                                    &model,
                                    &compact_retry_request_from_responses(&payload, &replay_plan),
                                )
                                .await
                            {
                                transparent_compaction_attempted = true;
                                compacted_retry_active = true;
                                continuation_recovery = false;
                                payload.input = Value::Array(compacted_output);
                                payload.extra.remove("previous_response_id");
                                queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                                tracing::info!(
                                    account_id = %lease.account_id,
                                    principal_id = %principal_id,
                                    "retrying responses stream on same lease after hidden context-length compaction"
                                );
                                continue;
                            }
                            handle_hidden_failure(&state, &lease, kind).await;
                            if should_retry_hidden_failure(
                                kind,
                                non_quota_failovers,
                                max_non_quota_failovers,
                            ) {
                                note_retry_attempt(&mut attempt, &mut non_quota_failovers, kind);
                                selection = Some(
                                    match resolve_http_selection_or_wait(
                                        &state,
                                        &selection_request,
                                        &request_log,
                                        &routing,
                                        &context,
                                        subagent_count,
                                        stream_requested,
                                        state.config.heartbeat_seconds,
                                    )
                                    .await
                                    {
                                        Ok(selection) => selection,
                                        Err(response) => return response,
                                    },
                                );
                                continue;
                            }
                            state.discard_pending_context_turn(&principal_id).await;
                            return gateway_failure_response(
                                stream_requested,
                                state.config.heartbeat_seconds,
                                gateway_failure_reason_from_upstream(kind),
                            );
                        }
                    }
                }
                match if codex_protocol {
                    upstream_stream_to_json_response(response, &model).await
                } else {
                    upstream_json_response(response, &model).await
                } {
                    ForwardOutcome::Response(success) => {
                        let ForwardSuccess {
                            response,
                            output_summary,
                            usage,
                            observed_model,
                            response_id,
                            response_output_items,
                        } = success;
                        let _ = state
                            .record_route_event(
                                &lease.account_id,
                                RouteEventRequest {
                                    mode: lease.route_mode,
                                    kind: "success".to_string(),
                                },
                            )
                            .await;
                        state
                            .record_context_output_with_response(
                                &principal_id,
                                success_response_summary(
                                    output_summary,
                                    &response_output_items,
                                    "assistant response delivered",
                                ),
                                response_id,
                                response_output_items,
                            )
                            .await;
                        state
                            .record_request_log(build_request_log_entry(
                                &request_log,
                                &lease,
                                response.status().as_u16(),
                                usage,
                                observed_model,
                            ))
                            .await;
                        return response;
                    }
                    ForwardOutcome::HiddenFailure(kind) => {
                        if previous_response_id.is_some()
                            && kind == UpstreamFailureKind::Continuation
                            && !continuation_recovery
                        {
                            continuation_recovery = true;
                            compacted_retry_active = false;
                            payload.extra.remove("previous_response_id");
                            queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                            tracing::info!(
                                account_id = %lease.account_id,
                                principal_id = %principal_id,
                                "retrying responses request on same lease after continuation recovery fallback"
                            );
                            continue;
                        }
                        if !transparent_compaction_attempted
                            && should_transparent_compact_hidden_failure(kind)
                            && let Some(compacted_output) = compact_retry_output_items(
                                &state,
                                &credential,
                                &lease,
                                &context,
                                &model,
                                &compact_retry_request_from_responses(&payload, &replay_plan),
                            )
                            .await
                        {
                            transparent_compaction_attempted = true;
                            compacted_retry_active = true;
                            continuation_recovery = false;
                            payload.input = Value::Array(compacted_output);
                            payload.extra.remove("previous_response_id");
                            queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                            tracing::info!(
                                account_id = %lease.account_id,
                                principal_id = %principal_id,
                                "retrying responses request on same lease after hidden context-length compaction"
                            );
                            continue;
                        }
                        handle_hidden_failure(&state, &lease, kind).await;
                        if should_retry_hidden_failure(
                            kind,
                            non_quota_failovers,
                            max_non_quota_failovers,
                        ) {
                            note_retry_attempt(&mut attempt, &mut non_quota_failovers, kind);
                            selection = Some(
                                match resolve_http_selection_or_wait(
                                    &state,
                                    &selection_request,
                                    &request_log,
                                    &routing,
                                    &context,
                                    subagent_count,
                                    stream_requested,
                                    state.config.heartbeat_seconds,
                                )
                                .await
                                {
                                    Ok(selection) => selection,
                                    Err(response) => return response,
                                },
                            );
                            continue;
                        }
                        state.discard_pending_context_turn(&principal_id).await;
                        return gateway_failure_response(
                            stream_requested,
                            state.config.heartbeat_seconds,
                            gateway_failure_reason_from_upstream(kind),
                        );
                    }
                }
            }
            Err(error) => {
                handle_upstream_failure(&state, &lease, &error).await;
                tracing::warn!(
                    account_id = %lease.account_id,
                    route_mode = %lease.route_mode.as_str(),
                    status = ?error.status,
                    kind = ?error.kind,
                    failure_subkind = error.subkind_label(),
                    reset_at = ?error.reset_at,
                    cf_ray = ?error.cf_ray,
                    body_preview = %truncate_text(error.body.clone().unwrap_or_default(), 160),
                    "responses upstream request failed"
                );
                if previous_response_id.is_some()
                    && error.kind == UpstreamFailureKind::Continuation
                    && !continuation_recovery
                {
                    continuation_recovery = true;
                    compacted_retry_active = false;
                    payload.extra.remove("previous_response_id");
                    queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                    tracing::info!(
                        account_id = %lease.account_id,
                        principal_id = %principal_id,
                        "retrying responses request on same lease after continuation rejection"
                    );
                    continue;
                }
                if !transparent_compaction_attempted
                    && should_passthrough_compact_upstream_error(&error)
                    && let Some(compacted_output) = compact_retry_output_items(
                        &state,
                        &credential,
                        &lease,
                        &context,
                        &model,
                        &compact_retry_request_from_responses(&payload, &replay_plan),
                    )
                    .await
                {
                    transparent_compaction_attempted = true;
                    compacted_retry_active = true;
                    continuation_recovery = false;
                    payload.input = Value::Array(compacted_output);
                    payload.extra.remove("previous_response_id");
                    queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                    tracing::info!(
                        account_id = %lease.account_id,
                        principal_id = %principal_id,
                        "retrying responses request on same lease after transparent compaction"
                    );
                    continue;
                }
                if should_retry_upstream_failure(
                    &error,
                    non_quota_failovers,
                    max_non_quota_failovers,
                ) {
                    note_retry_attempt(&mut attempt, &mut non_quota_failovers, error.kind);
                    selection = Some(
                        match resolve_http_selection_or_wait(
                            &state,
                            &selection_request,
                            &request_log,
                            &routing,
                            &context,
                            subagent_count,
                            stream_requested,
                            state.config.heartbeat_seconds,
                        )
                        .await
                        {
                            Ok(selection) => selection,
                            Err(response) => return response,
                        },
                    );
                    continue;
                }
                state.discard_pending_context_turn(&principal_id).await;
                return gateway_failure_response(
                    stream_requested,
                    state.config.heartbeat_seconds,
                    gateway_failure_reason_from_upstream(error.kind),
                );
            }
        }
    }
}

async fn responses_compact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ResponsesRequest>,
) -> Response<Body> {
    let Some(auth) = authenticated_context(&state, &headers).await else {
        return unauthorized().into_response();
    };
    let subagent_count = headers.get("x-openai-subagent").map(|_| 1_u32).unwrap_or(0);
    let requested_model = payload.model.clone();
    let effective_model = resolve_effective_model(&auth.api_key, &requested_model);
    log_model_resolution(&auth.api_key, &requested_model, &effective_model);
    let effective_reasoning_effort =
        resolve_effective_reasoning_for_responses(&auth.api_key, &payload);
    let payload = apply_responses_policy(
        &payload,
        &effective_model,
        effective_reasoning_effort.as_deref(),
    );
    let model = payload.model.clone();
    let routing = match resolve_request_scope(&state, &auth, &headers, &model).await {
        Ok(routing) => routing,
        Err(error) => return invalid_request(&error.message).into_response(),
    };
    let principal_id = routing.principal_id.clone();
    let cache_affinity_key =
        responses_cache_affinity_key(auth.tenant.id, cache_continuity_anchor(&routing), &payload);
    let request_log = RequestLogSeed {
        api_key: auth.api_key.clone(),
        tenant_id: auth.tenant.id,
        principal_id: principal_id.clone(),
        endpoint: "/v1/responses/compact",
        method: "POST",
        requested_model,
        effective_model: model.clone(),
        reasoning_effort: effective_reasoning_effort.clone(),
    };
    let selection_request = LeaseSelectionRequest {
        tenant_id: auth.tenant.id,
        principal_id: routing.lease_principal_id.clone(),
        model: model.clone(),
        reasoning_effort: effective_reasoning_effort,
        subagent_count,
        cache_affinity_key,
        placement_affinity_key: routing.placement_affinity_key.clone(),
    };
    let context = forward_context(&headers, &routing);
    let mut selection = Some(
        match resolve_http_selection_or_wait(
            &state,
            &selection_request,
            &request_log,
            &routing,
            &context,
            subagent_count,
            false,
            state.config.heartbeat_seconds,
        )
        .await
        {
            Ok(selection) => selection,
            Err(response) => return response,
        },
    );

    let mut attempt = 0usize;
    let mut non_quota_failovers = 0usize;
    let max_non_quota_failovers = non_quota_retry_budget(&state, &selection_request).await;
    loop {
        let Some((lease, _prompt_cache_key, _warmup)) = selection.take() else {
            return gateway_failure_response(
                false,
                state.config.heartbeat_seconds,
                GatewayFailureReason::UpstreamFailure,
            );
        };
        let execution_guard = state
            .acquire_execution_guard(auth.tenant.id, &lease.account_id, &model)
            .await;
        let Some(credential) = state.credential_for_account(&lease.account_id).await else {
            let _ = state
                .failover_account(&lease.account_id, "credential-missing", 300, true)
                .await;
            tracing::warn!(
                account_id = %lease.account_id,
                principal_id = %principal_id,
                "selected account missing credential for responses compact, retrying hidden failover"
            );
            if non_quota_failovers == 0 {
                attempt += 1;
                non_quota_failovers += 1;
                selection = Some(
                    match resolve_http_selection_or_wait(
                        &state,
                        &selection_request,
                        &request_log,
                        &routing,
                        &context,
                        subagent_count,
                        false,
                        state.config.heartbeat_seconds,
                    )
                    .await
                    {
                        Ok(selection) => selection,
                        Err(response) => return response,
                    },
                );
                continue;
            }
            return gateway_failure_response(
                false,
                state.config.heartbeat_seconds,
                GatewayFailureReason::UpstreamFailure,
            );
        };
        let upstream_value = compact_payload_for_upstream(&payload);
        let fallback_upstream_value = compact_payload_for_standard_responses(&payload);

        match state
            .upstream
            .post_json(
                &credential,
                "responses/compact",
                &upstream_value,
                &context,
                false,
                lease.route_mode,
            )
            .await
        {
            Ok(response) => match upstream_json_response(response, &model).await {
                ForwardOutcome::Response(success) => {
                    let ForwardSuccess {
                        response,
                        usage,
                        observed_model,
                        ..
                    } = success;
                    let _execution_guard = execution_guard;
                    let _ = state
                        .record_route_event(
                            &lease.account_id,
                            RouteEventRequest {
                                mode: lease.route_mode,
                                kind: "success".to_string(),
                            },
                        )
                        .await;
                    state
                        .record_request_log(build_request_log_entry(
                            &request_log,
                            &lease,
                            response.status().as_u16(),
                            usage,
                            observed_model,
                        ))
                        .await;
                    return response;
                }
                ForwardOutcome::HiddenFailure(kind) => {
                    let _execution_guard = execution_guard;
                    handle_hidden_failure(&state, &lease, kind).await;
                    if should_retry_hidden_failure(
                        kind,
                        non_quota_failovers,
                        max_non_quota_failovers,
                    ) {
                        note_retry_attempt(&mut attempt, &mut non_quota_failovers, kind);
                        selection = Some(
                            match resolve_http_selection_or_wait(
                                &state,
                                &selection_request,
                                &request_log,
                                &routing,
                                &context,
                                subagent_count,
                                false,
                                state.config.heartbeat_seconds,
                            )
                            .await
                            {
                                Ok(selection) => selection,
                                Err(response) => return response,
                            },
                        );
                        continue;
                    }
                    return gateway_failure_response(
                        false,
                        state.config.heartbeat_seconds,
                        gateway_failure_reason_from_upstream(kind),
                    );
                }
            },
            Err(error) => {
                let _execution_guard = execution_guard;
                let mut terminal_error = error;
                tracing::warn!(
                    account_id = %lease.account_id,
                    route_mode = %lease.route_mode.as_str(),
                    status = ?terminal_error.status,
                    kind = ?terminal_error.kind,
                    failure_subkind = terminal_error.subkind_label(),
                    reset_at = ?terminal_error.reset_at,
                    cf_ray = ?terminal_error.cf_ray,
                    body_preview = %truncate_text(terminal_error.body.clone().unwrap_or_default(), 160),
                    "responses compact upstream request failed"
                );
                if should_retry_compact_with_standard_responses(
                    &credential.base_url,
                    &terminal_error,
                ) {
                    tracing::info!(
                        account_id = %lease.account_id,
                        route_mode = %lease.route_mode.as_str(),
                        status = ?terminal_error.status,
                        kind = ?terminal_error.kind,
                        "retrying responses compact on same account via standard responses fallback"
                    );
                    match state
                        .upstream
                        .post_json(
                            &credential,
                            "responses",
                            &fallback_upstream_value,
                            &context,
                            false,
                            lease.route_mode,
                        )
                        .await
                    {
                        Ok(response) => match upstream_json_response(response, &model).await {
                            ForwardOutcome::Response(success) => {
                                let ForwardSuccess {
                                    response,
                                    usage,
                                    observed_model,
                                    ..
                                } = success;
                                let _ = state
                                    .record_route_event(
                                        &lease.account_id,
                                        RouteEventRequest {
                                            mode: lease.route_mode,
                                            kind: "success".to_string(),
                                        },
                                    )
                                    .await;
                                state
                                    .record_request_log(build_request_log_entry(
                                        &request_log,
                                        &lease,
                                        response.status().as_u16(),
                                        usage,
                                        observed_model,
                                    ))
                                    .await;
                                return response;
                            }
                            ForwardOutcome::HiddenFailure(kind) => {
                                terminal_error = UpstreamFailure {
                                    status: None,
                                    body: None,
                                    kind,
                                    subkind: None,
                                    cf_ray: None,
                                    reset_at: None,
                                };
                            }
                        },
                        Err(fallback_error) => {
                            tracing::warn!(
                                account_id = %lease.account_id,
                                route_mode = %lease.route_mode.as_str(),
                                status = ?fallback_error.status,
                                kind = ?fallback_error.kind,
                                failure_subkind = fallback_error.subkind_label(),
                                reset_at = ?fallback_error.reset_at,
                                cf_ray = ?fallback_error.cf_ray,
                                body_preview = %truncate_text(
                                    fallback_error.body.clone().unwrap_or_default(),
                                    160
                                ),
                                "responses compact fallback to standard responses also failed"
                            );
                            terminal_error = fallback_error;
                        }
                    }
                }

                handle_upstream_failure(&state, &lease, &terminal_error).await;
                if should_retry_upstream_failure(
                    &terminal_error,
                    non_quota_failovers,
                    max_non_quota_failovers,
                ) {
                    note_retry_attempt(&mut attempt, &mut non_quota_failovers, terminal_error.kind);
                    selection = Some(
                        match resolve_http_selection_or_wait(
                            &state,
                            &selection_request,
                            &request_log,
                            &routing,
                            &context,
                            subagent_count,
                            false,
                            state.config.heartbeat_seconds,
                        )
                        .await
                        {
                            Ok(selection) => selection,
                            Err(response) => return response,
                        },
                    );
                    continue;
                }
                if should_passthrough_compact_upstream_error(&terminal_error) {
                    return passthrough_upstream_error_response(
                        terminal_error.status.unwrap_or(StatusCode::BAD_GATEWAY),
                        terminal_error.body.unwrap_or_default(),
                    );
                }
                return gateway_failure_response(
                    false,
                    state.config.heartbeat_seconds,
                    gateway_failure_reason_from_upstream(terminal_error.kind),
                );
            }
        }
    }
}

async fn responses_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    let Some(auth) = authenticated_context(&state, &headers).await else {
        return unauthorized().into_response();
    };
    let subagent_count = headers.get("x-openai-subagent").map(|_| 1_u32).unwrap_or(0);
    ws.on_upgrade(move |socket| async move {
        handle_responses_ws(socket, state, auth, headers, subagent_count).await;
    })
    .into_response()
}

async fn handle_responses_ws(
    mut socket: WebSocket,
    state: AppState,
    auth: GatewayAuthContext,
    headers: HeaderMap,
    subagent_count: u32,
) {
    loop {
        let Some(message) = socket.recv().await else {
            break;
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(%error, "responses websocket receive failed");
                break;
            }
        };

        match message {
            Message::Text(text) => {
                if handle_responses_ws_text(
                    &mut socket,
                    &state,
                    &auth,
                    &headers,
                    subagent_count,
                    text.as_str(),
                )
                .await
                .is_err()
                {
                    break;
                }
            }
            Message::Binary(_) => {
                if send_ws_failure_response(
                    &mut socket,
                    None,
                    "invalid_request_error",
                    "binary websocket frames are not supported on /v1/responses",
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

async fn handle_responses_ws_text(
    socket: &mut WebSocket,
    state: &AppState,
    auth: &GatewayAuthContext,
    headers: &HeaderMap,
    subagent_count: u32,
    message: &str,
) -> Result<(), ()> {
    let payload = match parse_responses_ws_create(message) {
        Ok(payload) => payload,
        Err(error) => {
            send_ws_failure_response(socket, None, "invalid_request_error", &error).await?;
            return Ok(());
        }
    };

    forward_responses_ws(socket, state, auth, headers, subagent_count, payload).await
}

async fn forward_responses_ws(
    socket: &mut WebSocket,
    state: &AppState,
    auth: &GatewayAuthContext,
    headers: &HeaderMap,
    subagent_count: u32,
    mut payload: ResponsesRequest,
) -> Result<(), ()> {
    let generate = match extract_ws_generate(&mut payload) {
        Ok(generate) => generate,
        Err(error) => {
            send_ws_failure_response(socket, None, "invalid_request_error", &error).await?;
            return Ok(());
        }
    };
    if !generate {
        send_ws_empty_response(socket, &payload.model).await?;
        return Ok(());
    }

    let requested_model = payload.model.clone();
    let effective_model = resolve_effective_model(&auth.api_key, &requested_model);
    log_model_resolution(&auth.api_key, &requested_model, &effective_model);
    let effective_reasoning_effort =
        resolve_effective_reasoning_for_responses(&auth.api_key, &payload);
    let mut payload = apply_responses_policy(
        &payload,
        &effective_model,
        effective_reasoning_effort.as_deref(),
    );
    payload.stream = Some(true);
    payload.extra.remove("background");

    let model = payload.model.clone();
    let routing = match resolve_request_scope(state, auth, headers, &model).await {
        Ok(routing) => routing,
        Err(error) => {
            send_ws_failure_response(socket, None, "invalid_request_error", &error.message).await?;
            return Ok(());
        }
    };
    let principal_id = routing.principal_id.clone();
    let cache_affinity_key =
        responses_cache_affinity_key(auth.tenant.id, cache_continuity_anchor(&routing), &payload);
    let scoped_context = forward_context(headers, &routing);
    let input_summary = summarize_value(&payload.input);
    let request_log = RequestLogSeed {
        api_key: auth.api_key.clone(),
        tenant_id: auth.tenant.id,
        principal_id: principal_id.clone(),
        endpoint: "/v1/responses",
        method: "WS",
        requested_model,
        effective_model: model.clone(),
        reasoning_effort: effective_reasoning_effort.clone(),
    };
    let selection_request = LeaseSelectionRequest {
        tenant_id: auth.tenant.id,
        principal_id: routing.lease_principal_id.clone(),
        model: model.clone(),
        reasoning_effort: effective_reasoning_effort.clone(),
        subagent_count,
        cache_affinity_key: cache_affinity_key.clone(),
        placement_affinity_key: routing.placement_affinity_key.clone(),
    };
    let mut recorded_input = false;
    let mut queued_same_lease: Option<(CliLease, String)> = None;
    let mut continuation_recovery = false;
    let mut transparent_compaction_attempted = false;
    let mut compacted_retry_active = false;
    let mut selection = Some(
        resolve_ws_selection_or_wait(
            socket,
            state,
            &selection_request,
            &request_log,
            &routing,
            &scoped_context,
            subagent_count,
            state.config.heartbeat_seconds,
        )
        .await?,
    );

    let mut attempt = 0usize;
    let mut non_quota_failovers = 0usize;
    let max_non_quota_failovers = non_quota_retry_budget(&state, &selection_request).await;
    loop {
        let Some((lease, prompt_cache_key)) = queued_same_lease.take().or_else(|| {
            selection
                .take()
                .map(|(lease, prompt_cache_key, _warmup)| (lease, prompt_cache_key))
        }) else {
            send_ws_gateway_failure_response(socket, GatewayFailureReason::UpstreamFailure).await?;
            return Ok(());
        };
        let execution_guard = state
            .acquire_execution_guard(auth.tenant.id, &lease.account_id, &model)
            .await;
        if !recorded_input {
            state
                .begin_context_turn(
                    &principal_id,
                    &model,
                    lease.generation,
                    input_summary.clone(),
                    normalize_responses_input(payload.input.clone()),
                )
                .await;
            recorded_input = true;
        }
        let Some(credential) = state.credential_for_account(&lease.account_id).await else {
            let _ = state
                .failover_account(&lease.account_id, "credential-missing", 300, true)
                .await;
            tracing::warn!(
                account_id = %lease.account_id,
                principal_id = %principal_id,
                "selected account missing credential for responses websocket, retrying hidden failover"
            );
            if non_quota_failovers == 0 {
                attempt += 1;
                non_quota_failovers += 1;
                selection = Some(
                    resolve_ws_selection_or_wait(
                        socket,
                        state,
                        &selection_request,
                        &request_log,
                        &routing,
                        &scoped_context,
                        subagent_count,
                        state.config.heartbeat_seconds,
                    )
                    .await?,
                );
                continue;
            }
            state.discard_pending_context_turn(&principal_id).await;
            send_ws_gateway_failure_response(socket, GatewayFailureReason::UpstreamFailure).await?;
            return Ok(());
        };
        let codex_protocol = is_codex_chatgpt_backend(&credential.base_url);
        let previous_response_id = payload
            .extra
            .get("previous_response_id")
            .and_then(Value::as_str);
        let replay_plan = if continuation_recovery {
            state
                .continuation_recovery_plan_for_request(&principal_id, previous_response_id)
                .await
        } else if compacted_retry_active {
            ReplayPlan::default()
        } else {
            state
                .replay_plan_for_request(&principal_id, lease.generation, previous_response_id)
                .await
        };
        log_request_attempt(
            &request_log,
            &routing,
            &scoped_context,
            &lease,
            replay_plan.fallback_text.as_deref(),
            attempt,
        );
        let replayed_tool_calls =
            if codex_protocol && replay_plan.input_items.is_empty() && !compacted_retry_active {
                state
                    .replay_tool_call_items_for(
                        &principal_id,
                        previous_response_id,
                        &responses_input_function_call_output_call_ids(&payload.input),
                    )
                    .await
            } else {
                Vec::new()
            };
        let upstream_value = responses_payload_for_upstream(
            &payload,
            prompt_cache_key.clone(),
            &replay_plan,
            codex_protocol,
            &replayed_tool_calls,
        );

        match state
            .upstream
            .post_json(
                &credential,
                "responses",
                &upstream_value,
                &scoped_context,
                true,
                lease.route_mode,
            )
            .await
        {
            Ok(response) => match upstream_stream_to_ws_response(
                response,
                socket,
                state,
                lease.clone(),
                request_log.clone(),
                principal_id.clone(),
                &model,
                execution_guard,
                state.config.heartbeat_seconds,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(kind) => {
                    if previous_response_id.is_some()
                        && kind == UpstreamFailureKind::Continuation
                        && !continuation_recovery
                    {
                        continuation_recovery = true;
                        compacted_retry_active = false;
                        payload.extra.remove("previous_response_id");
                        queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                        tracing::info!(
                            account_id = %lease.account_id,
                            principal_id = %principal_id,
                            "retrying responses websocket on same lease after continuation recovery fallback"
                        );
                        continue;
                    }
                    if !transparent_compaction_attempted
                        && should_transparent_compact_hidden_failure(kind)
                        && let Some(compacted_output) = compact_retry_output_items(
                            state,
                            &credential,
                            &lease,
                            &scoped_context,
                            &model,
                            &compact_retry_request_from_responses(&payload, &replay_plan),
                        )
                        .await
                    {
                        transparent_compaction_attempted = true;
                        compacted_retry_active = true;
                        continuation_recovery = false;
                        payload.input = Value::Array(compacted_output);
                        payload.extra.remove("previous_response_id");
                        queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                        tracing::info!(
                            account_id = %lease.account_id,
                            principal_id = %principal_id,
                            "retrying responses websocket on same lease after hidden context-length compaction"
                        );
                        continue;
                    }
                    handle_hidden_failure(state, &lease, kind).await;
                    if should_retry_hidden_failure(
                        kind,
                        non_quota_failovers,
                        max_non_quota_failovers,
                    ) {
                        note_retry_attempt(&mut attempt, &mut non_quota_failovers, kind);
                        selection = Some(
                            resolve_ws_selection_or_wait(
                                socket,
                                state,
                                &selection_request,
                                &request_log,
                                &routing,
                                &scoped_context,
                                subagent_count,
                                state.config.heartbeat_seconds,
                            )
                            .await?,
                        );
                        continue;
                    }
                    state.discard_pending_context_turn(&principal_id).await;
                    send_ws_gateway_failure_response(
                        socket,
                        gateway_failure_reason_from_upstream(kind),
                    )
                    .await?;
                    return Ok(());
                }
            },
            Err(error) => {
                handle_upstream_failure(state, &lease, &error).await;
                tracing::warn!(
                    account_id = %lease.account_id,
                    route_mode = %lease.route_mode.as_str(),
                    status = ?error.status,
                    kind = ?error.kind,
                    failure_subkind = error.subkind_label(),
                    reset_at = ?error.reset_at,
                    cf_ray = ?error.cf_ray,
                    body_preview = %truncate_text(error.body.clone().unwrap_or_default(), 160),
                    "responses websocket upstream request failed"
                );
                if previous_response_id.is_some()
                    && error.kind == UpstreamFailureKind::Continuation
                    && !continuation_recovery
                {
                    continuation_recovery = true;
                    compacted_retry_active = false;
                    payload.extra.remove("previous_response_id");
                    queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                    tracing::info!(
                        account_id = %lease.account_id,
                        principal_id = %principal_id,
                        "retrying responses websocket on same lease after continuation rejection"
                    );
                    continue;
                }
                if !transparent_compaction_attempted
                    && should_passthrough_compact_upstream_error(&error)
                    && let Some(compacted_output) = compact_retry_output_items(
                        state,
                        &credential,
                        &lease,
                        &scoped_context,
                        &model,
                        &compact_retry_request_from_responses(&payload, &replay_plan),
                    )
                    .await
                {
                    transparent_compaction_attempted = true;
                    compacted_retry_active = true;
                    continuation_recovery = false;
                    payload.input = Value::Array(compacted_output);
                    payload.extra.remove("previous_response_id");
                    queued_same_lease = Some((lease.clone(), prompt_cache_key.clone()));
                    tracing::info!(
                        account_id = %lease.account_id,
                        principal_id = %principal_id,
                        "retrying responses websocket on same lease after transparent compaction"
                    );
                    continue;
                }
                if should_retry_upstream_failure(
                    &error,
                    non_quota_failovers,
                    max_non_quota_failovers,
                ) {
                    note_retry_attempt(&mut attempt, &mut non_quota_failovers, error.kind);
                    selection = Some(
                        resolve_ws_selection_or_wait(
                            socket,
                            state,
                            &selection_request,
                            &request_log,
                            &routing,
                            &scoped_context,
                            subagent_count,
                            state.config.heartbeat_seconds,
                        )
                        .await?,
                    );
                    continue;
                }
                state.discard_pending_context_turn(&principal_id).await;
                send_ws_gateway_failure_response(
                    socket,
                    gateway_failure_reason_from_upstream(error.kind),
                )
                .await?;
                return Ok(());
            }
        }
    }
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChatCompletionsRequest>,
) -> Response<Body> {
    let Some(auth) = authenticated_context(&state, &headers).await else {
        return unauthorized().into_response();
    };
    let subagent_count = headers.get("x-openai-subagent").map(|_| 1_u32).unwrap_or(0);
    let requested_model = payload.model.clone();
    let effective_model = resolve_effective_model(&auth.api_key, &requested_model);
    log_model_resolution(&auth.api_key, &requested_model, &effective_model);
    let effective_reasoning_effort = resolve_effective_reasoning_for_chat(&auth.api_key, &payload);
    let payload = apply_chat_policy(
        &payload,
        &effective_model,
        effective_reasoning_effort.as_deref(),
    );
    let model = payload.model.clone();
    let routing = match resolve_request_scope(&state, &auth, &headers, &model).await {
        Ok(routing) => routing,
        Err(error) => return invalid_request(&error.message).into_response(),
    };
    let principal_id = routing.principal_id.clone();
    let cache_affinity_key =
        chat_cache_affinity_key(auth.tenant.id, cache_continuity_anchor(&routing), &payload);
    let message_summary = summarize_messages(&payload.messages);
    let request_log = RequestLogSeed {
        api_key: auth.api_key.clone(),
        tenant_id: auth.tenant.id,
        principal_id: principal_id.clone(),
        endpoint: "/v1/chat/completions",
        method: "POST",
        requested_model,
        effective_model: model.clone(),
        reasoning_effort: effective_reasoning_effort.clone(),
    };
    let selection_request = LeaseSelectionRequest {
        tenant_id: auth.tenant.id,
        principal_id: routing.lease_principal_id.clone(),
        model: model.clone(),
        reasoning_effort: payload.reasoning_effort.clone(),
        subagent_count,
        cache_affinity_key: cache_affinity_key.clone(),
        placement_affinity_key: routing.placement_affinity_key.clone(),
    };
    let context = forward_context(&headers, &routing);
    let stream_requested = payload.stream.unwrap_or(false);
    let mut recorded_input = false;
    let mut transparent_compaction_attempted = false;
    let mut compacted_retry_input: Option<Vec<Value>> = None;
    let mut selection = Some(
        match resolve_http_selection_or_wait(
            &state,
            &selection_request,
            &request_log,
            &routing,
            &context,
            subagent_count,
            stream_requested,
            state.config.heartbeat_seconds,
        )
        .await
        {
            Ok(selection) => selection,
            Err(response) => return response,
        },
    );

    let mut attempt = 0usize;
    let mut non_quota_failovers = 0usize;
    let max_non_quota_failovers = non_quota_retry_budget(&state, &selection_request).await;
    loop {
        let Some((lease, prompt_cache_key, _warmup)) = selection.take() else {
            return gateway_chat_failure_response(
                stream_requested,
                state.config.heartbeat_seconds,
                &payload.model,
                GatewayFailureReason::UpstreamFailure,
            );
        };
        if !recorded_input {
            state
                .begin_context_turn(
                    &principal_id,
                    &model,
                    lease.generation,
                    message_summary.clone(),
                    payload
                        .messages
                        .iter()
                        .map(chat_message_to_responses_input)
                        .collect::<Vec<_>>(),
                )
                .await;
            recorded_input = true;
        }
        let Some(credential) = state.credential_for_account(&lease.account_id).await else {
            let _ = state
                .failover_account(&lease.account_id, "credential-missing", 300, true)
                .await;
            tracing::warn!(
                account_id = %lease.account_id,
                principal_id = %principal_id,
                "selected account missing credential for chat adapter, retrying hidden failover"
            );
            if non_quota_failovers == 0 {
                attempt += 1;
                non_quota_failovers += 1;
                selection = Some(
                    match resolve_http_selection_or_wait(
                        &state,
                        &selection_request,
                        &request_log,
                        &routing,
                        &context,
                        subagent_count,
                        stream_requested,
                        state.config.heartbeat_seconds,
                    )
                    .await
                    {
                        Ok(selection) => selection,
                        Err(response) => return response,
                    },
                );
                continue;
            }
            state.discard_pending_context_turn(&principal_id).await;
            return gateway_chat_failure_response(
                stream_requested,
                state.config.heartbeat_seconds,
                &payload.model,
                GatewayFailureReason::UpstreamFailure,
            );
        };
        let codex_protocol = is_codex_chatgpt_backend(&credential.base_url);
        let replay_plan = if compacted_retry_input.is_some() {
            ReplayPlan::default()
        } else {
            state
                .replay_plan_for_request(&principal_id, lease.generation, None)
                .await
        };
        log_request_attempt(
            &request_log,
            &routing,
            &context,
            &lease,
            replay_plan.fallback_text.as_deref(),
            attempt,
        );
        let upstream_value = if let Some(compacted_input) = compacted_retry_input.clone() {
            responses_payload_from_compacted_chat_input(
                &payload,
                compacted_input,
                prompt_cache_key,
                codex_protocol,
            )
        } else {
            responses_payload_from_chat_request(
                &payload,
                prompt_cache_key,
                &replay_plan,
                codex_protocol,
            )
        };
        let upstream_stream = codex_protocol || stream_requested;

        match state
            .upstream
            .post_json(
                &credential,
                "responses",
                &upstream_value,
                &context,
                upstream_stream,
                lease.route_mode,
            )
            .await
        {
            Ok(response) => {
                if stream_requested {
                    let near_quota_guard = state.near_quota_guard_enabled(&lease.account_id).await;
                    match upstream_responses_stream_to_chat_response(
                        response,
                        state.clone(),
                        lease.clone(),
                        request_log.clone(),
                        principal_id.clone(),
                        &payload.model,
                        near_quota_guard,
                        state.config.heartbeat_seconds,
                    )
                    .await
                    {
                        ForwardOutcome::Response(success) => return success.response,
                        ForwardOutcome::HiddenFailure(kind) => {
                            if !transparent_compaction_attempted
                                && should_transparent_compact_hidden_failure(kind)
                                && let Some(compacted_output) = compact_retry_output_items(
                                    &state,
                                    &credential,
                                    &lease,
                                    &context,
                                    &payload.model,
                                    &compact_retry_request_from_chat(&payload, &replay_plan),
                                )
                                .await
                            {
                                transparent_compaction_attempted = true;
                                compacted_retry_input = Some(compacted_output);
                                tracing::info!(
                                    account_id = %lease.account_id,
                                    principal_id = %principal_id,
                                    "retrying chat stream on same lease after hidden context-length compaction"
                                );
                                continue;
                            }
                            handle_hidden_failure(&state, &lease, kind).await;
                            if should_retry_hidden_failure(
                                kind,
                                non_quota_failovers,
                                max_non_quota_failovers,
                            ) {
                                note_retry_attempt(&mut attempt, &mut non_quota_failovers, kind);
                                selection = Some(
                                    match resolve_http_selection_or_wait(
                                        &state,
                                        &selection_request,
                                        &request_log,
                                        &routing,
                                        &context,
                                        subagent_count,
                                        stream_requested,
                                        state.config.heartbeat_seconds,
                                    )
                                    .await
                                    {
                                        Ok(selection) => selection,
                                        Err(response) => return response,
                                    },
                                );
                                continue;
                            }
                            state.discard_pending_context_turn(&principal_id).await;
                            return gateway_chat_failure_response(
                                stream_requested,
                                state.config.heartbeat_seconds,
                                &payload.model,
                                gateway_failure_reason_from_upstream(kind),
                            );
                        }
                    }
                }
                match if codex_protocol {
                    upstream_stream_to_chat_json_response(response, &payload.model).await
                } else {
                    upstream_responses_json_to_chat_response(response, &payload.model).await
                } {
                    ForwardOutcome::Response(success) => {
                        let ForwardSuccess {
                            response,
                            output_summary,
                            usage,
                            observed_model,
                            response_id,
                            response_output_items,
                        } = success;
                        let _ = state
                            .record_route_event(
                                &lease.account_id,
                                RouteEventRequest {
                                    mode: lease.route_mode,
                                    kind: "success".to_string(),
                                },
                            )
                            .await;
                        state
                            .record_context_output_with_response(
                                &principal_id,
                                success_response_summary(
                                    output_summary,
                                    &response_output_items,
                                    "assistant response delivered",
                                ),
                                response_id,
                                response_output_items,
                            )
                            .await;
                        state
                            .record_request_log(build_request_log_entry(
                                &request_log,
                                &lease,
                                response.status().as_u16(),
                                usage,
                                observed_model,
                            ))
                            .await;
                        return response;
                    }
                    ForwardOutcome::HiddenFailure(kind) => {
                        if !transparent_compaction_attempted
                            && should_transparent_compact_hidden_failure(kind)
                            && let Some(compacted_output) = compact_retry_output_items(
                                &state,
                                &credential,
                                &lease,
                                &context,
                                &payload.model,
                                &compact_retry_request_from_chat(&payload, &replay_plan),
                            )
                            .await
                        {
                            transparent_compaction_attempted = true;
                            compacted_retry_input = Some(compacted_output);
                            tracing::info!(
                                account_id = %lease.account_id,
                                principal_id = %principal_id,
                                "retrying chat request on same lease after hidden context-length compaction"
                            );
                            continue;
                        }
                        handle_hidden_failure(&state, &lease, kind).await;
                        if should_retry_hidden_failure(
                            kind,
                            non_quota_failovers,
                            max_non_quota_failovers,
                        ) {
                            note_retry_attempt(&mut attempt, &mut non_quota_failovers, kind);
                            selection = Some(
                                match resolve_http_selection_or_wait(
                                    &state,
                                    &selection_request,
                                    &request_log,
                                    &routing,
                                    &context,
                                    subagent_count,
                                    stream_requested,
                                    state.config.heartbeat_seconds,
                                )
                                .await
                                {
                                    Ok(selection) => selection,
                                    Err(response) => return response,
                                },
                            );
                            continue;
                        }
                        state.discard_pending_context_turn(&principal_id).await;
                        return gateway_chat_failure_response(
                            stream_requested,
                            state.config.heartbeat_seconds,
                            &payload.model,
                            gateway_failure_reason_from_upstream(kind),
                        );
                    }
                }
            }
            Err(error) => {
                handle_upstream_failure(&state, &lease, &error).await;
                tracing::warn!(
                    account_id = %lease.account_id,
                    route_mode = %lease.route_mode.as_str(),
                    status = ?error.status,
                    kind = ?error.kind,
                    failure_subkind = error.subkind_label(),
                    reset_at = ?error.reset_at,
                    cf_ray = ?error.cf_ray,
                    body_preview = %truncate_text(error.body.clone().unwrap_or_default(), 160),
                    "chat upstream request failed"
                );
                if !transparent_compaction_attempted
                    && should_passthrough_compact_upstream_error(&error)
                    && let Some(compacted_output) = compact_retry_output_items(
                        &state,
                        &credential,
                        &lease,
                        &context,
                        &payload.model,
                        &compact_retry_request_from_chat(&payload, &replay_plan),
                    )
                    .await
                {
                    transparent_compaction_attempted = true;
                    compacted_retry_input = Some(compacted_output);
                    tracing::info!(
                        account_id = %lease.account_id,
                        principal_id = %principal_id,
                        "retrying chat request on same lease after transparent compaction"
                    );
                    continue;
                }
                if should_retry_upstream_failure(
                    &error,
                    non_quota_failovers,
                    max_non_quota_failovers,
                ) {
                    note_retry_attempt(&mut attempt, &mut non_quota_failovers, error.kind);
                    selection = Some(
                        match resolve_http_selection_or_wait(
                            &state,
                            &selection_request,
                            &request_log,
                            &routing,
                            &context,
                            subagent_count,
                            stream_requested,
                            state.config.heartbeat_seconds,
                        )
                        .await
                        {
                            Ok(selection) => selection,
                            Err(response) => return response,
                        },
                    );
                    continue;
                }
                state.discard_pending_context_turn(&principal_id).await;
                return gateway_chat_failure_response(
                    stream_requested,
                    state.config.heartbeat_seconds,
                    &payload.model,
                    gateway_failure_reason_from_upstream(error.kind),
                );
            }
        }
    }
}

async fn authenticated_context(
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

fn resolve_effective_model(api_key: &GatewayApiKey, requested_model: &str) -> String {
    let requested_model = requested_model.trim();
    let default_model = api_key
        .default_model
        .clone()
        .filter(|value| !value.trim().is_empty());
    if api_key.force_model_override {
        return default_model.unwrap_or_else(|| requested_model.to_string());
    }

    codex_worker_model_fallback(requested_model, default_model.as_deref())
        .unwrap_or_else(|| requested_model.to_string())
}

fn codex_worker_model_fallback(
    requested_model: &str,
    default_model: Option<&str>,
) -> Option<String> {
    let normalized = requested_model.trim().to_ascii_lowercase();
    let trimmed_default = default_model
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let default_model_or = |fallback: &str| {
        trimmed_default
            .map(str::to_string)
            .or_else(|| Some(fallback.to_string()))
    };
    let codex_default_or = |fallback: &str| {
        trimmed_default
            .filter(|value| value.to_ascii_lowercase().contains("codex"))
            .map(str::to_string)
            .or_else(|| Some(fallback.to_string()))
    };
    if normalized == "gpt-5.1-codex-mini"
        || normalized.starts_with("gpt-5.1-codex-mini-")
        || normalized.contains("-codex-mini-")
    {
        return codex_default_or("gpt-5.3-codex");
    }
    if matches!(
        normalized.as_str(),
        "gpt-5" | "gpt-5.1" | "gpt-5.1-low" | "gpt-5.1-medium" | "gpt-5.1-high"
    ) {
        return default_model_or("gpt-5.4");
    }
    if normalized == "gpt-5-codex"
        || normalized == "gpt-5.1-codex"
        || normalized.starts_with("gpt-5.1-codex-")
    {
        return codex_default_or("gpt-5.3-codex");
    }
    None
}

fn log_model_resolution(api_key: &GatewayApiKey, requested_model: &str, effective_model: &str) {
    if requested_model == effective_model {
        return;
    }
    let resolution_reason = if api_key.force_model_override {
        "forced_api_key_override"
    } else {
        "codex_model_alias_fallback"
    };
    tracing::info!(
        requested_model = requested_model,
        effective_model = effective_model,
        resolution_reason,
        default_model = ?api_key.default_model,
        force_model_override = api_key.force_model_override,
        "resolved gateway effective model"
    );
}

fn resolve_effective_reasoning_for_chat(
    api_key: &GatewayApiKey,
    payload: &ChatCompletionsRequest,
) -> Option<String> {
    if api_key.force_reasoning_effort {
        return api_key.reasoning_effort.clone();
    }
    payload.reasoning_effort.clone()
}

fn resolve_effective_reasoning_for_responses(
    api_key: &GatewayApiKey,
    payload: &ResponsesRequest,
) -> Option<String> {
    if api_key.force_reasoning_effort {
        return api_key.reasoning_effort.clone();
    }
    payload
        .reasoning
        .as_ref()
        .and_then(|reasoning| reasoning.get("effort"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn apply_chat_policy(
    payload: &ChatCompletionsRequest,
    effective_model: &str,
    reasoning_effort: Option<&str>,
) -> ChatCompletionsRequest {
    let mut next = payload.clone();
    next.model = effective_model.to_string();
    next.reasoning_effort = reasoning_effort.map(str::to_string);
    next
}

fn apply_responses_policy(
    payload: &ResponsesRequest,
    effective_model: &str,
    reasoning_effort: Option<&str>,
) -> ResponsesRequest {
    let mut next = payload.clone();
    next.model = effective_model.to_string();
    if let Some(level) = reasoning_effort {
        let mut reasoning = next
            .reasoning
            .take()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        reasoning.insert("effort".to_string(), Value::String(level.to_string()));
        next.reasoning = Some(Value::Object(reasoning));
    }
    next
}

fn build_request_log_entry(
    seed: &RequestLogSeed,
    lease: &CliLease,
    status_code: u16,
    usage: RequestLogUsage,
    observed_model: Option<String>,
) -> RequestLogEntry {
    let effective_model = observed_model.unwrap_or_else(|| seed.effective_model.clone());
    let estimated_cost_usd = crate::pricing::estimate_cost_usd(&effective_model, &usage);
    RequestLogEntry {
        id: format!("log_{}", uuid::Uuid::new_v4().simple()),
        api_key_id: seed.api_key.id,
        tenant_id: seed.tenant_id,
        user_name: seed.api_key.name.clone(),
        user_email: seed.api_key.email.clone(),
        principal_id: seed.principal_id.clone(),
        account_id: lease.account_id.clone(),
        account_label: lease.account_label.clone(),
        method: seed.method.to_string(),
        endpoint: seed.endpoint.to_string(),
        requested_model: seed.requested_model.clone(),
        effective_model,
        reasoning_effort: seed.reasoning_effort.clone(),
        route_mode: lease.route_mode,
        status_code,
        usage,
        estimated_cost_usd,
        created_at: Utc::now(),
    }
}

fn merge_request_usage(target: &mut RequestLogUsage, next: RequestLogUsage) {
    target.input_tokens = target.input_tokens.max(next.input_tokens);
    target.cached_input_tokens = target.cached_input_tokens.max(next.cached_input_tokens);
    target.output_tokens = target.output_tokens.max(next.output_tokens);
    target.total_tokens = target.total_tokens.max(next.total_tokens);
}

fn request_usage_from_value(value: &Value) -> RequestLogUsage {
    let input_tokens = usage_value(value, &["input_tokens", "prompt_tokens"]);
    let output_tokens = usage_value(value, &["output_tokens", "completion_tokens"]);
    let total_tokens = usage_value(value, &["total_tokens"]).max(input_tokens + output_tokens);
    let cached_input_tokens = usage_object(value)
        .and_then(|usage| {
            usage
                .get("input_tokens_details")
                .and_then(Value::as_object)
                .and_then(|details| {
                    details
                        .get("cached_tokens")
                        .or_else(|| details.get("cached_input_tokens"))
                })
                .and_then(Value::as_u64)
                .or_else(|| {
                    usage
                        .get("prompt_tokens_details")
                        .and_then(Value::as_object)
                        .and_then(|details| {
                            details
                                .get("cached_tokens")
                                .or_else(|| details.get("cached_input_tokens"))
                        })
                        .and_then(Value::as_u64)
                })
                .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64))
        })
        .unwrap_or_default();

    RequestLogUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        total_tokens,
    }
}

fn observed_model_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get("openai-model")
        .or_else(|| headers.get("x-openai-model"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn observed_model_from_value(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("model"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

impl GatewayFailureReason {
    fn as_reason(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Quota => "quota",
            Self::Capability => "capability",
            Self::UpstreamFailure => "upstream_failure",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Queue => "Gateway queue active.",
            Self::Quota => "Gateway upstream quota exhausted.",
            Self::Capability => "Gateway capability match unavailable.",
            Self::UpstreamFailure => "Gateway upstream request failed.",
        }
    }
}

fn gateway_failure_reason_from_upstream(kind: UpstreamFailureKind) -> GatewayFailureReason {
    match kind {
        UpstreamFailureKind::Quota => GatewayFailureReason::Quota,
        UpstreamFailureKind::Capability => GatewayFailureReason::Capability,
        UpstreamFailureKind::RateLimited => GatewayFailureReason::UpstreamFailure,
        UpstreamFailureKind::Cf
        | UpstreamFailureKind::Auth
        | UpstreamFailureKind::Length
        | UpstreamFailureKind::Continuation
        | UpstreamFailureKind::Generic => GatewayFailureReason::UpstreamFailure,
    }
}

fn should_retry_via_selection(
    kind: UpstreamFailureKind,
    non_quota_failovers: usize,
    max_non_quota_failovers: usize,
) -> bool {
    kind.requires_failover()
        && (kind == UpstreamFailureKind::Quota || non_quota_failovers < max_non_quota_failovers)
}

fn is_retryable_generic_status(status: Option<StatusCode>) -> bool {
    match status {
        None => true,
        Some(status) => {
            status.is_server_error()
                || matches!(
                    status,
                    StatusCode::REQUEST_TIMEOUT
                        | StatusCode::BAD_GATEWAY
                        | StatusCode::SERVICE_UNAVAILABLE
                        | StatusCode::GATEWAY_TIMEOUT
                )
        }
    }
}

fn should_retry_hidden_failure(
    kind: UpstreamFailureKind,
    non_quota_failovers: usize,
    max_non_quota_failovers: usize,
) -> bool {
    if kind == UpstreamFailureKind::Generic {
        return non_quota_failovers < max_non_quota_failovers;
    }
    should_retry_via_selection(kind, non_quota_failovers, max_non_quota_failovers)
}

fn should_retry_upstream_failure(
    error: &UpstreamFailure,
    non_quota_failovers: usize,
    max_non_quota_failovers: usize,
) -> bool {
    if error.kind == UpstreamFailureKind::Generic {
        return non_quota_failovers < max_non_quota_failovers
            && is_retryable_generic_status(error.status);
    }
    should_retry_via_selection(error.kind, non_quota_failovers, max_non_quota_failovers)
}

async fn non_quota_retry_budget(
    state: &AppState,
    selection_request: &LeaseSelectionRequest,
) -> usize {
    let accounts = state.runtime.accounts.read().await;
    let credentials = state.runtime.credentials.read().await;
    let matching_accounts = accounts
        .values()
        .filter(|account| {
            account.tenant_id == selection_request.tenant_id
                && credentials.contains_key(account.id.as_str())
                && account
                    .models
                    .iter()
                    .any(|model| model == &selection_request.model)
        })
        .count();
    non_quota_retry_budget_for_matching_accounts(matching_accounts)
}

fn non_quota_retry_budget_for_matching_accounts(matching_accounts: usize) -> usize {
    match matching_accounts {
        0 => 0,
        1 => 1,
        count => count.saturating_sub(1),
    }
}

fn note_retry_attempt(
    attempt: &mut usize,
    non_quota_failovers: &mut usize,
    kind: UpstreamFailureKind,
) {
    *attempt += 1;
    if kind != UpstreamFailureKind::Quota {
        *non_quota_failovers += 1;
    }
}

fn gateway_failure_reason_from_selection(
    kind: LeaseSelectionExhaustedKind,
) -> GatewayFailureReason {
    match kind {
        LeaseSelectionExhaustedKind::QuotaExhausted => GatewayFailureReason::Quota,
        LeaseSelectionExhaustedKind::Capability => GatewayFailureReason::Capability,
        LeaseSelectionExhaustedKind::Cooldown => GatewayFailureReason::Queue,
        LeaseSelectionExhaustedKind::Unavailable => GatewayFailureReason::UpstreamFailure,
    }
}

fn selection_wait_interval(
    earliest_retry_at: Option<chrono::DateTime<Utc>>,
    earliest_reset_at: Option<chrono::DateTime<Utc>>,
) -> Duration {
    let fallback = Duration::from_secs(5);
    let now = Utc::now();
    earliest_retry_at
        .or(earliest_reset_at)
        .and_then(|reset_at| {
            if reset_at <= now {
                Some(Duration::from_secs(1))
            } else {
                (reset_at - now).to_std().ok()
            }
        })
        .map(|duration| duration.min(fallback))
        .unwrap_or(fallback)
}

async fn resolve_http_selection_or_wait(
    state: &AppState,
    selection_request: &LeaseSelectionRequest,
    request_log: &RequestLogSeed,
    routing: &RequestRoutingScope,
    context: &ForwardContext,
    subagent_count: u32,
    stream_requested: bool,
    heartbeat_seconds: u64,
) -> Result<
    (
        CliLease,
        String,
        crate::scheduler::token_optimizer::WarmupDecision,
    ),
    Response<Body>,
> {
    loop {
        match state.resolve_lease_outcome(selection_request.clone()).await {
            LeaseSelectionOutcome::Selected(lease, prompt_cache_key, warmup) => {
                return Ok((lease, prompt_cache_key, warmup));
            }
            LeaseSelectionOutcome::Exhausted(exhausted)
                if matches!(
                    exhausted.kind,
                    LeaseSelectionExhaustedKind::QuotaExhausted
                        | LeaseSelectionExhaustedKind::Cooldown
                ) =>
            {
                log_selection_exhausted(request_log, routing, context, subagent_count, &exhausted);
                tokio::time::sleep(selection_wait_interval(
                    exhausted.earliest_retry_at,
                    exhausted.earliest_reset_at,
                ))
                .await;
            }
            LeaseSelectionOutcome::Exhausted(exhausted) => {
                log_selection_exhausted(request_log, routing, context, subagent_count, &exhausted);
                return Err(gateway_failure_response(
                    stream_requested,
                    heartbeat_seconds,
                    gateway_failure_reason_from_selection(exhausted.kind),
                ));
            }
        }
    }
}

async fn resolve_ws_selection_or_wait(
    socket: &mut WebSocket,
    state: &AppState,
    selection_request: &LeaseSelectionRequest,
    request_log: &RequestLogSeed,
    routing: &RequestRoutingScope,
    context: &ForwardContext,
    subagent_count: u32,
    heartbeat_seconds: u64,
) -> Result<
    (
        CliLease,
        String,
        crate::scheduler::token_optimizer::WarmupDecision,
    ),
    (),
> {
    loop {
        match state.resolve_lease_outcome(selection_request.clone()).await {
            LeaseSelectionOutcome::Selected(lease, prompt_cache_key, warmup) => {
                return Ok((lease, prompt_cache_key, warmup));
            }
            LeaseSelectionOutcome::Exhausted(exhausted)
                if matches!(
                    exhausted.kind,
                    LeaseSelectionExhaustedKind::QuotaExhausted
                        | LeaseSelectionExhaustedKind::Cooldown
                ) =>
            {
                log_selection_exhausted(request_log, routing, context, subagent_count, &exhausted);
                socket
                    .send(Message::Ping(Bytes::new()))
                    .await
                    .map_err(|_| ())?;
                tokio::time::sleep(
                    selection_wait_interval(
                        exhausted.earliest_retry_at,
                        exhausted.earliest_reset_at,
                    )
                    .min(Duration::from_secs(heartbeat_seconds.max(1))),
                )
                .await;
            }
            LeaseSelectionOutcome::Exhausted(exhausted) => {
                log_selection_exhausted(request_log, routing, context, subagent_count, &exhausted);
                send_ws_gateway_failure_response(
                    socket,
                    gateway_failure_reason_from_selection(exhausted.kind),
                )
                .await?;
                return Err(());
            }
        }
    }
}

fn gateway_error_value(reason: GatewayFailureReason) -> Value {
    json!({
        "code": "server_busy",
        "message": reason.message(),
        "type": "server_busy",
        "reason": reason.as_reason(),
        "retryable": true
    })
}

fn response_failure_value(response_id: Option<&str>, reason: GatewayFailureReason) -> Value {
    let response_id = response_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("resp_failed_{}", uuid::Uuid::new_v4().simple()));
    json!({
        "type": "response.failed",
        "response": {
            "id": response_id,
            "status": "failed",
            "error": gateway_error_value(reason),
        }
    })
}

fn response_failed_sse_bytes(response_id: Option<&str>, reason: GatewayFailureReason) -> Bytes {
    Bytes::from(format!(
        "event: response.failed\ndata: {}\n\n",
        response_failure_value(response_id, reason)
    ))
}

fn gateway_failure_response(
    stream_requested: bool,
    heartbeat_seconds: u64,
    reason: GatewayFailureReason,
) -> Response<Body> {
    if stream_requested {
        let response_id = format!("resp_failed_{}", uuid::Uuid::new_v4().simple());
        let error = gateway_error_value(reason);
        let wait_stream = stream! {
            yield Ok::<Event, Infallible>(
                Event::default()
                    .event("response.created")
                    .data(json!({
                        "type": "response.created",
                        "response": {
                            "id": response_id,
                            "status": "in_progress"
                        }
                    }).to_string())
            );
            yield Ok::<Event, Infallible>(
                Event::default()
                    .event("response.failed")
                    .data(json!({
                        "type": "response.failed",
                        "response": {
                            "id": response_id,
                            "status": "failed",
                            "error": error,
                        }
                    }).to_string())
            );
        };
        return Sse::new(wait_stream)
            .keep_alive(
                KeepAlive::new()
                    .interval(Duration::from_secs(heartbeat_seconds))
                    .text("heartbeat"),
            )
            .into_response();
    }

    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": gateway_error_value(reason)
        })),
    )
        .into_response()
}

fn passthrough_upstream_error_response(status: StatusCode, body: String) -> Response<Body> {
    let response_body = if body.is_empty() {
        Body::from(
            status
                .canonical_reason()
                .unwrap_or("upstream error")
                .to_string(),
        )
    } else {
        Body::from(body.clone())
    };
    let content_type = if body.trim_start().starts_with('{') || body.trim_start().starts_with('[') {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(response_body)
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")))
}

fn should_retry_compact_with_standard_responses(base_url: &str, error: &UpstreamFailure) -> bool {
    if !is_codex_chatgpt_backend(base_url) {
        return false;
    }
    match error.kind {
        UpstreamFailureKind::Generic | UpstreamFailureKind::Cf => true,
        UpstreamFailureKind::Capability => error
            .status
            .map(|status| {
                matches!(
                    status,
                    StatusCode::BAD_REQUEST
                        | StatusCode::NOT_FOUND
                        | StatusCode::METHOD_NOT_ALLOWED
                        | StatusCode::NOT_IMPLEMENTED
                        | StatusCode::SERVICE_UNAVAILABLE
                        | StatusCode::BAD_GATEWAY
                        | StatusCode::GATEWAY_TIMEOUT
                )
            })
            .unwrap_or(true),
        UpstreamFailureKind::Auth
        | UpstreamFailureKind::Quota
        | UpstreamFailureKind::RateLimited
        | UpstreamFailureKind::Length
        | UpstreamFailureKind::Continuation => false,
    }
}

fn should_passthrough_compact_upstream_error(error: &UpstreamFailure) -> bool {
    let Some(status) = error.status else {
        return false;
    };
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return true;
    }
    if status.is_client_error() && !error.kind.requires_failover() {
        return true;
    }
    let lowered = error
        .body
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        "context_length_exceeded",
        "context window",
        "input too large",
        "payload too large",
        "request body too large",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn should_transparent_compact_hidden_failure(kind: UpstreamFailureKind) -> bool {
    kind == UpstreamFailureKind::Length
}

fn gateway_chat_failure_response(
    stream_requested: bool,
    heartbeat_seconds: u64,
    model: &str,
    reason: GatewayFailureReason,
) -> Response<Body> {
    let _ = stream_requested;
    let _ = heartbeat_seconds;
    let _ = model;
    gateway_failure_response(false, heartbeat_seconds, reason)
}

fn parse_responses_ws_create(message: &str) -> Result<ResponsesRequest, String> {
    let mut value =
        serde_json::from_str::<Value>(message).map_err(|error| format!("invalid JSON: {error}"))?;
    let Some(object) = value.as_object_mut() else {
        return Err("invalid websocket payload: expected a JSON object".to_string());
    };
    let Some(event_type) = object
        .remove("type")
        .and_then(|value| value.as_str().map(str::to_string))
    else {
        return Err("invalid websocket payload: missing type".to_string());
    };
    if event_type != "response.create" {
        return Err(format!("unsupported websocket event type: {event_type}"));
    }
    object.remove("background");
    serde_json::from_value(value)
        .map_err(|error| format!("invalid response.create payload: {error}"))
}

fn extract_ws_generate(payload: &mut ResponsesRequest) -> Result<bool, String> {
    match payload.extra.remove("generate") {
        None => Ok(true),
        Some(Value::Bool(generate)) => Ok(generate),
        Some(_) => Err("invalid response.create payload: generate must be a boolean".to_string()),
    }
}

async fn upstream_stream_to_ws_response(
    response: reqwest::Response,
    socket: &mut WebSocket,
    state: &AppState,
    lease: CliLease,
    request_log: RequestLogSeed,
    principal_id: String,
    expected_model: &str,
    execution_guard: Option<crate::state::AccountExecutionGuard>,
    heartbeat_seconds: u64,
) -> Result<(), UpstreamFailureKind> {
    let status = response.status();
    let headers = response.headers().clone();
    if let Some(kind) = hidden_failure_kind_from_headers(&headers, expected_model) {
        return Err(kind);
    }

    let _execution_guard = execution_guard;
    let (mut upstream, buffered_records, mut buffer) =
        preflight_response_stream(response, expected_model).await?;
    let mut output_summary = String::new();
    let mut usage = RequestLogUsage::default();
    let mut observed_model = observed_model_from_headers(&headers);
    let mut streamed_response_id = None;
    let mut streamed_output_items = BTreeMap::new();
    let mut completed_response = None;
    let mut had_hidden_failure = false;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(heartbeat_seconds.max(1)));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let _ = heartbeat.tick().await;

    for record in buffered_records {
        if let Some(value) = response_value_from_sse_record(&record) {
            merge_request_usage(&mut usage, request_usage_from_value(&value));
            if observed_model.is_none() {
                observed_model = observed_model_from_value(&value);
            }
            capture_stream_response_metadata(
                &value,
                &mut streamed_response_id,
                &mut streamed_output_items,
                &mut completed_response,
            );
        }
        if let Some(delta) = response_delta_text(&record) {
            output_summary.push_str(delta.as_str());
        }
        send_ws_record(socket, &record)
            .await
            .map_err(|_| UpstreamFailureKind::Generic)?;
    }

    loop {
        tokio::select! {
                maybe_chunk = upstream.next() => {
                    let Some(chunk) = maybe_chunk else {
                        break;
                    };
                    let Ok(chunk) = chunk else {
                        had_hidden_failure = true;
                        handle_hidden_failure(state, &lease, UpstreamFailureKind::Generic).await;
                        send_ws_gateway_failure_response(
                            socket,
                            GatewayFailureReason::UpstreamFailure,
                        )
                        .await
                        .map_err(|_| UpstreamFailureKind::Generic)?;
                        break;
                    };
                    buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
                    while let Some(record) = take_sse_record(&mut buffer) {
                        let hidden_kind = hidden_failure_kind_from_sse_record(&record, expected_model);
                        if let Some(value) = response_value_from_sse_record(&record) {
                            merge_request_usage(&mut usage, request_usage_from_value(&value));
                            if observed_model.is_none() {
                                observed_model = observed_model_from_value(&value);
                            }
                        capture_stream_response_metadata(
                            &value,
                            &mut streamed_response_id,
                            &mut streamed_output_items,
                            &mut completed_response,
                        );
                        }
                        if let Some(delta) = response_delta_text(&record) {
                            output_summary.push_str(delta.as_str());
                        }
                        send_ws_record(socket, &record)
                            .await
                            .map_err(|_| UpstreamFailureKind::Generic)?;
                        if let Some(kind) = hidden_kind {
                            had_hidden_failure = true;
                            handle_hidden_failure(state, &lease, kind).await;
                            break;
                        }
                    }
                    if had_hidden_failure {
                        break;
                    }
                }
                _ = heartbeat.tick() => {
                    socket
                        .send(Message::Ping(Bytes::new()))
                    .await
                    .map_err(|_| UpstreamFailureKind::Generic)?;
            }
        }
    }

    if had_hidden_failure {
        state.discard_pending_context_turn(&principal_id).await;
        return Ok(());
    }

    let _ = state
        .record_route_event(
            &lease.account_id,
            RouteEventRequest {
                mode: lease.route_mode,
                kind: "success".to_string(),
            },
        )
        .await;
    let summary = if output_summary.trim().is_empty() {
        "streamed assistant response delivered".to_string()
    } else {
        truncate_text(output_summary, 240)
    };
    let completed_response = finalize_stream_response(
        completed_response,
        streamed_response_id,
        streamed_output_items,
    );
    state
        .record_context_output_with_response(
            &principal_id,
            summary,
            completed_response.as_ref().and_then(response_id_from_value),
            completed_response
                .as_ref()
                .map(response_output_items_from_value)
                .unwrap_or_default(),
        )
        .await;
    state
        .record_request_log(build_request_log_entry(
            &request_log,
            &lease,
            status.as_u16(),
            usage,
            observed_model,
        ))
        .await;
    Ok(())
}

async fn send_ws_record(socket: &mut WebSocket, record: &str) -> Result<(), ()> {
    let Some((_, data)) = parse_sse_record(record) else {
        return Ok(());
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }
    socket
        .send(Message::Text(data.to_string().into()))
        .await
        .map_err(|_| ())
}

async fn send_ws_gateway_failure_response(
    socket: &mut WebSocket,
    reason: GatewayFailureReason,
) -> Result<(), ()> {
    send_ws_json(socket, &response_failure_value(None, reason)).await
}

async fn send_ws_empty_response(socket: &mut WebSocket, model: &str) -> Result<(), ()> {
    let response_id = format!("resp_ws_empty_{}", uuid::Uuid::new_v4().simple());
    send_ws_json(
        socket,
        &json!({
            "type": "response.created",
            "response": {
                "id": response_id,
                "model": model,
                "status": "in_progress",
                "output": [],
            }
        }),
    )
    .await?;
    send_ws_json(
        socket,
        &json!({
            "type": "response.completed",
            "response": {
                "id": response_id,
                "model": model,
                "status": "completed",
                "output": [],
            }
        }),
    )
    .await
}

async fn send_ws_failure_response(
    socket: &mut WebSocket,
    response_id: Option<&str>,
    code: &str,
    message: &str,
) -> Result<(), ()> {
    let mut response = serde_json::Map::new();
    if let Some(response_id) = response_id {
        response.insert("id".to_string(), Value::String(response_id.to_string()));
    }
    response.insert("status".to_string(), Value::String("failed".to_string()));
    response.insert(
        "error".to_string(),
        json!({
            "code": code,
            "message": message,
        }),
    );
    send_ws_json(
        socket,
        &Value::Object(serde_json::Map::from_iter([
            (
                "type".to_string(),
                Value::String("response.failed".to_string()),
            ),
            ("response".to_string(), Value::Object(response)),
        ])),
    )
    .await
}

async fn send_ws_json(socket: &mut WebSocket, value: &Value) -> Result<(), ()> {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
fn websocket_failure_response_event(reason: GatewayFailureReason) -> Value {
    response_failure_value(None, reason)
}

async fn upstream_json_response(
    response: reqwest::Response,
    expected_model: &str,
) -> ForwardOutcome {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.bytes().await.unwrap_or_default();
    let parsed = serde_json::from_slice::<Value>(&bytes).ok();
    if let Some(kind) = parsed
        .as_ref()
        .and_then(|value| hidden_failure_kind_from_json(value, expected_model, &headers))
        .or_else(|| {
            std::str::from_utf8(&bytes)
                .ok()
                .and_then(classify_failure_body)
        })
    {
        return ForwardOutcome::HiddenFailure(kind);
    }
    let output_summary = parsed
        .as_ref()
        .map(extract_response_output_text)
        .filter(|summary| !summary.is_empty());
    let usage = parsed
        .as_ref()
        .map(request_usage_from_value)
        .unwrap_or_default();
    let observed_model = parsed
        .as_ref()
        .and_then(observed_model_from_value)
        .or_else(|| observed_model_from_headers(&headers));
    let response_id = parsed.as_ref().and_then(response_id_from_value);
    let response_output_items = parsed
        .as_ref()
        .map(response_output_items_from_value)
        .unwrap_or_default();
    let mut builder = Response::builder().status(status);
    copy_upstream_headers(&mut builder, &headers);
    let response = builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")));
    ForwardOutcome::Response(ForwardSuccess {
        response,
        output_summary,
        usage,
        observed_model,
        response_id,
        response_output_items,
    })
}

async fn upstream_responses_json_to_chat_response(
    response: reqwest::Response,
    fallback_model: &str,
) -> ForwardOutcome {
    let status = response.status();
    let headers = response.headers().clone();
    let value = response
        .json::<Value>()
        .await
        .unwrap_or_else(|_| json!({"status":"incomplete"}));
    if let Some(kind) = hidden_failure_kind_from_json(&value, fallback_model, &headers) {
        return ForwardOutcome::HiddenFailure(kind);
    }
    let output_summary = extract_response_output_text(&value);
    let usage = request_usage_from_value(&value);
    let observed_model =
        observed_model_from_value(&value).or_else(|| observed_model_from_headers(&headers));
    let response_id = response_id_from_value(&value);
    let response_output_items = response_output_items_from_value(&value);
    let payload = responses_json_to_chat_completion(&value, fallback_model);
    let bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    let mut builder = Response::builder().status(status);
    copy_upstream_headers(&mut builder, &headers);
    let response = builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")));
    ForwardOutcome::Response(ForwardSuccess {
        response,
        output_summary: (!output_summary.is_empty()).then_some(output_summary),
        usage,
        observed_model,
        response_id,
        response_output_items,
    })
}

async fn upstream_stream_to_json_response(
    response: reqwest::Response,
    expected_model: &str,
) -> ForwardOutcome {
    let (status, headers, value) =
        match collect_completed_response_from_stream(response, expected_model).await {
            Ok(collected) => collected,
            Err(kind) => return ForwardOutcome::HiddenFailure(kind),
        };

    if let Some(kind) = hidden_failure_kind_from_json(&value, expected_model, &headers) {
        return ForwardOutcome::HiddenFailure(kind);
    };

    let output_summary = extract_response_output_text(&value);
    let usage = request_usage_from_value(&value);
    let observed_model =
        observed_model_from_value(&value).or_else(|| observed_model_from_headers(&headers));
    let response_id = response_id_from_value(&value);
    let response_output_items = response_output_items_from_value(&value);
    let bytes = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let mut builder = Response::builder().status(status);
    if let Some(out) = builder.headers_mut() {
        copy_upstream_headers_to_response(out, &headers);
        out.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    let response = builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")));
    ForwardOutcome::Response(ForwardSuccess {
        response,
        output_summary: (!output_summary.is_empty()).then_some(output_summary),
        usage,
        observed_model,
        response_id,
        response_output_items,
    })
}

async fn upstream_stream_to_chat_json_response(
    response: reqwest::Response,
    fallback_model: &str,
) -> ForwardOutcome {
    let (status, headers, value) =
        match collect_completed_response_from_stream(response, fallback_model).await {
            Ok(collected) => collected,
            Err(kind) => return ForwardOutcome::HiddenFailure(kind),
        };

    if let Some(kind) = hidden_failure_kind_from_json(&value, fallback_model, &headers) {
        return ForwardOutcome::HiddenFailure(kind);
    };

    let output_summary = extract_response_output_text(&value);
    let usage = request_usage_from_value(&value);
    let observed_model =
        observed_model_from_value(&value).or_else(|| observed_model_from_headers(&headers));
    let response_id = response_id_from_value(&value);
    let response_output_items = response_output_items_from_value(&value);
    let payload = responses_json_to_chat_completion(&value, fallback_model);
    let bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    let mut builder = Response::builder().status(status);
    if let Some(out) = builder.headers_mut() {
        copy_upstream_headers_to_response(out, &headers);
        out.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    let response = builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")));
    ForwardOutcome::Response(ForwardSuccess {
        response,
        output_summary: (!output_summary.is_empty()).then_some(output_summary),
        usage,
        observed_model,
        response_id,
        response_output_items,
    })
}

async fn upstream_stream_response(
    response: reqwest::Response,
    state: AppState,
    lease: CliLease,
    request_log: RequestLogSeed,
    principal_id: String,
    expected_model: &str,
    heartbeat_seconds: u64,
    execution_guard: Option<crate::state::AccountExecutionGuard>,
) -> ForwardOutcome {
    let status = response.status();
    let headers = response.headers().clone();
    if let Some(kind) = hidden_failure_kind_from_headers(&headers, expected_model) {
        return ForwardOutcome::HiddenFailure(kind);
    }
    let expected_model = expected_model.to_string();
    let (upstream, buffered_records, initial_buffer) =
        match preflight_response_stream(response, &expected_model).await {
            Ok(preflight) => preflight,
            Err(kind) => return ForwardOutcome::HiddenFailure(kind),
        };
    let mut builder = Response::builder().status(status);
    copy_upstream_headers(&mut builder, &headers);
    let stream = stream! {
        let _execution_guard = execution_guard;
        let mut upstream = upstream;
        let mut buffer = initial_buffer;
        let mut had_hidden_failure = false;
        let mut output_summary = String::new();
        let mut usage = RequestLogUsage::default();
        let mut observed_model = observed_model_from_headers(&headers);
        let mut streamed_response_id = None;
        let mut streamed_output_items = BTreeMap::new();
        let mut completed_response = None;
        let mut heartbeat = tokio::time::interval(Duration::from_secs(heartbeat_seconds.max(1)));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let _ = heartbeat.tick().await;

        for record in buffered_records {
            if let Some(value) = response_value_from_sse_record(&record) {
                merge_request_usage(&mut usage, request_usage_from_value(&value));
                if observed_model.is_none() {
                    observed_model = observed_model_from_value(&value);
                }
                capture_stream_response_metadata(
                    &value,
                    &mut streamed_response_id,
                    &mut streamed_output_items,
                    &mut completed_response,
                );
            }
            if let Some(delta) = response_delta_text(&record) {
                output_summary.push_str(delta.as_str());
            }
            yield Ok::<Bytes, Infallible>(Bytes::from(format!("{record}\n\n")));
        }

        loop {
            tokio::select! {
                maybe_chunk = upstream.next() => {
                    let Some(chunk) = maybe_chunk else {
                        break;
                    };
                    let Ok(chunk) = chunk else {
                        had_hidden_failure = true;
                        handle_hidden_failure(&state, &lease, UpstreamFailureKind::Generic).await;
                        yield Ok::<Bytes, Infallible>(response_failed_sse_bytes(
                            streamed_response_id.as_deref(),
                            GatewayFailureReason::UpstreamFailure,
                        ));
                        break;
                    };
                    buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
                    while let Some(record) = take_sse_record(&mut buffer) {
                        let hidden_kind =
                            hidden_failure_kind_from_sse_record(&record, &expected_model);
                        if let Some(value) = response_value_from_sse_record(&record) {
                            merge_request_usage(&mut usage, request_usage_from_value(&value));
                            if observed_model.is_none() {
                                observed_model = observed_model_from_value(&value);
                            }
                            capture_stream_response_metadata(
                                &value,
                                &mut streamed_response_id,
                                &mut streamed_output_items,
                                &mut completed_response,
                            );
                        }
                        if let Some(delta) = response_delta_text(&record) {
                            output_summary.push_str(delta.as_str());
                        }
                        yield Ok::<Bytes, Infallible>(Bytes::from(format!("{record}\n\n")));
                        if let Some(kind) = hidden_kind {
                            had_hidden_failure = true;
                            handle_hidden_failure(&state, &lease, kind).await;
                            break;
                        }
                    }
                    if had_hidden_failure {
                        break;
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok::<Bytes, Infallible>(Bytes::from_static(b": heartbeat\n\n"));
                }
            }
        }

        if had_hidden_failure {
            state.discard_pending_context_turn(&principal_id).await;
        }
        if !had_hidden_failure {
            let _ = state
                .record_route_event(
                    &lease.account_id,
                    RouteEventRequest {
                        mode: lease.route_mode,
                        kind: "success".to_string(),
                    },
                )
                .await;
            let summary = if output_summary.trim().is_empty() {
                "streamed assistant response delivered".to_string()
            } else {
                truncate_text(output_summary, 240)
            };
            let completed_response = finalize_stream_response(
                completed_response,
                streamed_response_id,
                streamed_output_items,
            );
            state
                .record_context_output_with_response(
                    &principal_id,
                    summary,
                    completed_response.as_ref().and_then(response_id_from_value),
                    completed_response
                        .as_ref()
                        .map(response_output_items_from_value)
                        .unwrap_or_default(),
                )
                .await;
            state
                .record_request_log(build_request_log_entry(
                    &request_log,
                    &lease,
                    status.as_u16(),
                    usage,
                    observed_model,
                ))
                .await;
        }
    };
    let response = builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| Response::new(Body::from("upstream stream error")));
    ForwardOutcome::Response(ForwardSuccess {
        response,
        ..ForwardSuccess::default()
    })
}

async fn passthrough_stream_response(
    response: reqwest::Response,
    state: AppState,
    lease: CliLease,
    request_log: RequestLogSeed,
    principal_id: String,
    expected_model: &str,
    heartbeat_seconds: u64,
    execution_guard: Option<crate::state::AccountExecutionGuard>,
) -> ForwardOutcome {
    upstream_stream_response(
        response,
        state,
        lease,
        request_log,
        principal_id,
        expected_model,
        heartbeat_seconds,
        execution_guard,
    )
    .await
}

async fn upstream_responses_stream_to_chat_response(
    response: reqwest::Response,
    state: AppState,
    lease: CliLease,
    request_log: RequestLogSeed,
    principal_id: String,
    fallback_model: &str,
    near_quota_guard: bool,
    heartbeat_seconds: u64,
) -> ForwardOutcome {
    let status = response.status();
    let headers = response.headers().clone();
    if let Some(kind) = hidden_failure_kind_from_headers(&headers, fallback_model) {
        return ForwardOutcome::HiddenFailure(kind);
    }
    let (upstream, buffered_records, initial_buffer) = if near_quota_guard {
        match preflight_response_stream(response, fallback_model).await {
            Ok(preflight) => preflight,
            Err(kind) => return ForwardOutcome::HiddenFailure(kind),
        }
    } else {
        (
            Box::pin(response.bytes_stream())
                as Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
            Vec::new(),
            String::new(),
        )
    };
    let created = Utc::now().timestamp();
    let fallback_model = fallback_model.to_string();
    let headers_for_usage = headers.clone();
    let events = stream! {
        let gateway_state = state.clone();
        let mut stream = upstream;
        let mut buffer = initial_buffer;
        let mut adapter_state = ChatStreamAdapterState::new(&fallback_model, created);
        let mut had_hidden_failure = false;
        let mut usage = RequestLogUsage::default();
        let mut observed_model = observed_model_from_headers(&headers_for_usage);
        let mut streamed_response_id = None;
        let mut streamed_output_items = BTreeMap::new();
        let mut completed_response = None;
        for record in buffered_records {
            if let Some(value) = response_value_from_sse_record(&record) {
                merge_request_usage(&mut usage, request_usage_from_value(&value));
                if observed_model.is_none() {
                    observed_model = observed_model_from_value(&value);
                }
                capture_stream_response_metadata(
                    &value,
                    &mut streamed_response_id,
                    &mut streamed_output_items,
                    &mut completed_response,
                );
            }
            for event in translate_response_record_to_chat_events(&record, &mut adapter_state) {
                yield Ok::<Event, Infallible>(event);
            }
        }
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else {
                had_hidden_failure = true;
                handle_hidden_failure(&gateway_state, &lease, UpstreamFailureKind::Generic).await;
                for event in chat_gateway_failure_events(
                    &mut adapter_state,
                    UpstreamFailureKind::Generic,
                ) {
                    yield Ok::<Event, Infallible>(event);
                }
                break;
            };
            buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
            while let Some(record) = take_sse_record(&mut buffer) {
                let hidden_kind = hidden_failure_kind_from_sse_record(&record, &fallback_model);
                if let Some(value) = response_value_from_sse_record(&record) {
                    merge_request_usage(&mut usage, request_usage_from_value(&value));
                    if observed_model.is_none() {
                        observed_model = observed_model_from_value(&value);
                    }
                    capture_stream_response_metadata(
                        &value,
                        &mut streamed_response_id,
                        &mut streamed_output_items,
                        &mut completed_response,
                    );
                }
                for event in translate_response_record_to_chat_events(&record, &mut adapter_state) {
                    yield Ok::<Event, Infallible>(event);
                }
                if let Some(kind) = hidden_kind {
                    had_hidden_failure = true;
                    handle_hidden_failure(&gateway_state, &lease, kind).await;
                    if !adapter_state.finished {
                        for event in chat_gateway_failure_events(&mut adapter_state, kind) {
                            yield Ok::<Event, Infallible>(event);
                        }
                    }
                    break;
                }
            }
            if had_hidden_failure {
                break;
            }
        }
        if !adapter_state.finished && !had_hidden_failure {
            yield Ok::<Event, Infallible>(chat_completion_sse_event(chat_completion_chunk(
                &adapter_state.chat_id,
                &adapter_state.model,
                adapter_state.created,
                json!({}),
                Some("stop"),
            )));
            yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
        }
        if had_hidden_failure {
            gateway_state.discard_pending_context_turn(&principal_id).await;
        }
        if !had_hidden_failure {
            let _ = gateway_state
                .record_route_event(
                    &lease.account_id,
                    RouteEventRequest {
                        mode: lease.route_mode,
                        kind: "success".to_string(),
                    },
                )
                .await;
            let summary = if adapter_state.saw_tool_call {
                format!("streamed tool call response via {}", adapter_state.tool_calls.first().map(|tool| tool.name.as_str()).unwrap_or("tool"))
            } else {
                "streamed chat completion delivered".to_string()
            };
            let completed_response = finalize_stream_response(
                completed_response,
                streamed_response_id,
                streamed_output_items,
            );
            gateway_state
                .record_context_output_with_response(
                    &principal_id,
                    summary,
                    completed_response.as_ref().and_then(response_id_from_value),
                    completed_response
                        .as_ref()
                        .map(response_output_items_from_value)
                        .unwrap_or_default(),
                )
                .await;
            gateway_state
                .record_request_log(build_request_log_entry(
                    &request_log,
                    &lease,
                    status.as_u16(),
                    usage,
                    observed_model.or_else(|| Some(adapter_state.model.clone())),
                ))
                .await;
        }
    };

    let mut output = Sse::new(events)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(heartbeat_seconds))
                .text("heartbeat"),
        )
        .into_response();
    copy_upstream_headers_to_response(output.headers_mut(), &headers);
    ForwardOutcome::Response(ForwardSuccess {
        response: output,
        ..ForwardSuccess::default()
    })
}

async fn preflight_response_stream(
    response: reqwest::Response,
    expected_model: &str,
) -> Result<
    (
        Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
        Vec<String>,
        String,
    ),
    UpstreamFailureKind,
> {
    let mut upstream = response.bytes_stream();
    let mut buffer = String::new();
    let mut buffered_records = Vec::new();
    let mut saw_release_event = false;

    while !saw_release_event {
        let Some(chunk) = upstream.next().await else {
            break;
        };
        let chunk = chunk.map_err(|_| UpstreamFailureKind::Generic)?;
        buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
        while let Some(record) = take_sse_record(&mut buffer) {
            if let Some(kind) = hidden_failure_kind_from_sse_record(&record, expected_model) {
                return Err(kind);
            }
            let resolved_event = sse_resolved_event_name(&record);
            if !record.trim().is_empty() {
                buffered_records.push(record);
            }
            if resolved_event
                .as_deref()
                .is_some_and(is_stream_release_event)
            {
                saw_release_event = true;
                break;
            }
        }
    }

    Ok((Box::pin(upstream), buffered_records, buffer))
}

async fn collect_completed_response_from_stream(
    response: reqwest::Response,
    expected_model: &str,
) -> Result<(StatusCode, reqwest::header::HeaderMap, Value), UpstreamFailureKind> {
    let status = response.status();
    let headers = response.headers().clone();
    if let Some(kind) = hidden_failure_kind_from_headers(&headers, expected_model) {
        return Err(kind);
    }

    let mut upstream = response.bytes_stream();
    let mut buffer = String::new();
    let mut streamed_response_id = None;
    let mut streamed_output_items = BTreeMap::new();
    let mut completed_response = None;

    while let Some(chunk) = upstream.next().await {
        let chunk = chunk.map_err(|_| UpstreamFailureKind::Generic)?;
        buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
        while let Some(record) = take_sse_record(&mut buffer) {
            if let Some(kind) = hidden_failure_kind_from_sse_record(&record, expected_model) {
                return Err(kind);
            }
            if let Some(value) = response_value_from_sse_record(&record) {
                capture_stream_response_metadata(
                    &value,
                    &mut streamed_response_id,
                    &mut streamed_output_items,
                    &mut completed_response,
                );
            }
        }
    }

    if !buffer.trim().is_empty() {
        let record = buffer.trim();
        if let Some(kind) = hidden_failure_kind_from_sse_record(record, expected_model) {
            return Err(kind);
        }
        if let Some(value) = response_value_from_sse_record(record) {
            capture_stream_response_metadata(
                &value,
                &mut streamed_response_id,
                &mut streamed_output_items,
                &mut completed_response,
            );
        }
    }

    finalize_stream_response(
        completed_response,
        streamed_response_id,
        streamed_output_items,
    )
    .map(|value| (status, headers, value))
    .ok_or(UpstreamFailureKind::Generic)
}

async fn handle_hidden_failure(state: &AppState, lease: &CliLease, kind: UpstreamFailureKind) {
    match kind {
        UpstreamFailureKind::Cf => {
            let _ = state
                .record_route_event(
                    &lease.account_id,
                    RouteEventRequest {
                        mode: lease.route_mode,
                        kind: "cf_hit".to_string(),
                    },
                )
                .await;
        }
        UpstreamFailureKind::Auth => {
            let _ = state
                .failover_account(
                    &lease.account_id,
                    kind.severity(),
                    kind.cooldown_seconds(),
                    true,
                )
                .await;
        }
        UpstreamFailureKind::Quota => {
            let _ = state
                .mark_account_quota_exhausted(&lease.account_id, None, Some("hidden_quota"))
                .await;
        }
        UpstreamFailureKind::Length => {}
        UpstreamFailureKind::RateLimited | UpstreamFailureKind::Capability => {
            let _ = state
                .failover_account(
                    &lease.account_id,
                    kind.severity(),
                    kind.cooldown_seconds(),
                    false,
                )
                .await;
        }
        UpstreamFailureKind::Continuation => {}
        UpstreamFailureKind::Generic => {
            let _ = state
                .failover_account(&lease.account_id, "upstream_generic", 30, false)
                .await;
        }
    }
}

async fn handle_upstream_failure(state: &AppState, lease: &CliLease, error: &UpstreamFailure) {
    match error.kind {
        UpstreamFailureKind::Cf => {
            let _ = state
                .record_route_event(
                    &lease.account_id,
                    RouteEventRequest {
                        mode: lease.route_mode,
                        kind: "cf_hit".to_string(),
                    },
                )
                .await;
        }
        UpstreamFailureKind::Auth => {
            let _ = state
                .failover_account(
                    &lease.account_id,
                    error.kind.severity(),
                    error.kind.cooldown_seconds(),
                    true,
                )
                .await;
        }
        UpstreamFailureKind::Quota => {
            let reason = match error.subkind {
                Some(UpstreamFailureSubkind::Quota429) => Some("quota_429"),
                _ => Some("quota"),
            };
            let _ = state
                .mark_account_quota_exhausted(&lease.account_id, error.reset_at, reason)
                .await;
        }
        UpstreamFailureKind::Length => {}
        UpstreamFailureKind::RateLimited | UpstreamFailureKind::Capability => {
            let _ = state
                .failover_account(
                    &lease.account_id,
                    error.kind.severity(),
                    error.kind.cooldown_seconds(),
                    false,
                )
                .await;
        }
        UpstreamFailureKind::Continuation => {}
        UpstreamFailureKind::Generic => {
            if is_retryable_generic_status(error.status) {
                let _ = state
                    .failover_account(&lease.account_id, "upstream_generic", 30, false)
                    .await;
            }
        }
    }
}

fn hidden_failure_kind_from_headers(
    headers: &reqwest::header::HeaderMap,
    expected_model: &str,
) -> Option<UpstreamFailureKind> {
    let actual_model = headers
        .get("openai-model")
        .or_else(|| headers.get("x-openai-model"))
        .and_then(|value| value.to_str().ok());
    if actual_model.is_some_and(|actual| !model_matches_expected(actual, expected_model)) {
        return Some(UpstreamFailureKind::Capability);
    }
    None
}

fn hidden_failure_kind_from_json(
    value: &Value,
    expected_model: &str,
    headers: &reqwest::header::HeaderMap,
) -> Option<UpstreamFailureKind> {
    if hidden_failure_kind_from_headers(headers, expected_model).is_some() {
        return Some(UpstreamFailureKind::Capability);
    }
    if value
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|model| !model_matches_expected(model, expected_model))
    {
        return Some(UpstreamFailureKind::Capability);
    }
    if let Some(kind) = hidden_failure_kind_from_error_value(value) {
        return Some(kind);
    }
    if value
        .get("response")
        .and_then(|response| response.get("model"))
        .and_then(Value::as_str)
        .is_some_and(|model| !model_matches_expected(model, expected_model))
    {
        return Some(UpstreamFailureKind::Capability);
    }
    if value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("failed"))
        || value
            .get("response")
            .and_then(|response| response.get("status"))
            .and_then(Value::as_str)
            .is_some_and(|status| status.eq_ignore_ascii_case("failed"))
        || value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "response.failed")
    {
        return hidden_failure_kind_from_error_value(value).or(Some(UpstreamFailureKind::Generic));
    }
    None
}

fn hidden_failure_kind_from_error_value(value: &Value) -> Option<UpstreamFailureKind> {
    if let Some(error) = value.get("error").or_else(|| {
        value
            .get("response")
            .and_then(|response| response.get("error"))
    }) {
        return hidden_failure_kind_from_error_object(error);
    }
    if let Some(kind) = value
        .get("type")
        .and_then(Value::as_str)
        .and_then(hidden_failure_kind_from_code)
    {
        return Some(kind);
    }
    None
}

fn model_matches_expected(actual: &str, expected: &str) -> bool {
    actual == expected
        || actual
            .strip_prefix(expected)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit() || ch == '-')
            })
}

fn hidden_failure_kind_from_error_object(error: &Value) -> Option<UpstreamFailureKind> {
    for field in ["code", "type", "message"] {
        if let Some(kind) = error
            .get(field)
            .and_then(Value::as_str)
            .and_then(hidden_failure_kind_from_code)
        {
            return Some(kind);
        }
    }
    let serialized = summarize_value(error);
    classify_failure_body(&serialized)
}

fn hidden_failure_kind_from_code(value: &str) -> Option<UpstreamFailureKind> {
    let normalized = value.to_ascii_lowercase();
    if [
        "rate_limit_exceeded",
        "insufficient_quota",
        "usage_limit_reached",
        "usage_not_included",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
    {
        return Some(UpstreamFailureKind::Quota);
    }
    if [
        "invalid_api_key",
        "invalid_token",
        "token_expired",
        "unauthorized",
        "authentication",
        "proxy_auth_required",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
    {
        return Some(UpstreamFailureKind::Auth);
    }
    if [
        "context_length_exceeded",
        "context window",
        "maximum context length",
        "input too large",
        "payload too large",
        "request body too large",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
    {
        return Some(UpstreamFailureKind::Length);
    }
    if [
        "previous_response_id",
        "previous response id",
        "previous_response_not_found",
        "previous response not found",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
    {
        return Some(UpstreamFailureKind::Continuation);
    }
    if [
        "does not support",
        "unsupported",
        "unknown_model",
        "model_not_found",
        "reasoning_effort",
        "model_mismatch",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
    {
        return Some(UpstreamFailureKind::Capability);
    }
    if ["cloudflare", "cf-ray", "challenge-platform"]
        .iter()
        .any(|needle| normalized.contains(needle))
    {
        return Some(UpstreamFailureKind::Cf);
    }
    None
}

fn hidden_failure_kind_from_sse_record(
    record: &str,
    expected_model: &str,
) -> Option<UpstreamFailureKind> {
    let (_, data) = parse_sse_record(record)?;
    let value = serde_json::from_str::<Value>(&data).ok()?;
    hidden_failure_kind_from_json(&value, expected_model, &reqwest::header::HeaderMap::new())
}

fn sse_resolved_event_name(record: &str) -> Option<String> {
    let (event_name, data) = parse_sse_record(record)?;
    if !event_name.is_empty() {
        return Some(event_name);
    }
    serde_json::from_str::<Value>(&data).ok().and_then(|value| {
        value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn is_stream_release_event(event_name: &str) -> bool {
    matches!(
        event_name,
        "response.created"
            | "response.in_progress"
            | "response.output_text.delta"
            | "response.output_item.added"
            | "response.output_item.done"
            | "response.function_call_arguments.delta"
            | "response.function_call_arguments.done"
            | "response.reasoning_summary_text.delta"
            | "response.completed"
    )
}

fn response_value_from_sse_record(record: &str) -> Option<Value> {
    let (_, data) = parse_sse_record(record)?;
    serde_json::from_str::<Value>(&data).ok()
}

fn response_delta_text(record: &str) -> Option<String> {
    let value = response_value_from_sse_record(record)?;
    if value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "response.output_text.delta")
    {
        return value
            .get("delta")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "response.completed")
    {
        return (!extract_response_output_text(value.get("response").unwrap_or(&value)).is_empty())
            .then(|| extract_response_output_text(value.get("response").unwrap_or(&value)));
    }
    None
}

fn copy_upstream_headers(
    builder: &mut axum::http::response::Builder,
    headers: &reqwest::header::HeaderMap,
) {
    let Some(out) = builder.headers_mut() else {
        return;
    };
    for name in [
        "content-type",
        "cache-control",
        "x-request-id",
        "openai-model",
        "openai-processing-ms",
    ] {
        if let Some(value) = headers.get(name)
            && let Ok(header_name) = name.parse::<axum::http::HeaderName>()
        {
            out.insert(header_name, value.clone());
        }
    }
    if headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"))
    {
        out.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
    }
}

fn copy_upstream_headers_to_response(out: &mut HeaderMap, headers: &reqwest::header::HeaderMap) {
    for name in [
        "cache-control",
        "x-request-id",
        "openai-model",
        "openai-processing-ms",
    ] {
        if let Some(value) = headers.get(name)
            && let Ok(header_name) = name.parse::<axum::http::HeaderName>()
        {
            out.insert(header_name, value.clone());
        }
    }
}

fn prepend_context_messages(
    input: Vec<Value>,
    replay_prefix_items: &[Value],
    replay_context: Option<&str>,
) -> Value {
    let mut prefixed = Vec::new();
    prefixed.extend(replay_prefix_items.iter().cloned());
    if let Some(replay_context) = replay_context
        && !replay_context.trim().is_empty()
    {
        prefixed.push(replay_context_message(replay_context));
    }
    prefixed.extend(input);
    Value::Array(prefixed)
}

fn responses_cache_affinity_key(
    tenant_id: Uuid,
    continuity_anchor: Option<&str>,
    payload: &ResponsesRequest,
) -> String {
    exact_prefix_cache_affinity_key(
        tenant_id,
        continuity_anchor,
        &payload.model,
        payload.extra.get("instructions"),
        payload.extra.get("tools"),
        &normalize_responses_input(payload.input.clone()),
    )
}

fn chat_cache_affinity_key(
    tenant_id: Uuid,
    continuity_anchor: Option<&str>,
    payload: &ChatCompletionsRequest,
) -> String {
    let input = payload
        .messages
        .iter()
        .map(chat_message_to_responses_input)
        .collect::<Vec<_>>();
    exact_prefix_cache_affinity_key(
        tenant_id,
        continuity_anchor,
        &payload.model,
        payload.extra.get("instructions"),
        payload.extra.get("tools"),
        &input,
    )
}

fn exact_prefix_cache_affinity_key(
    tenant_id: Uuid,
    continuity_anchor: Option<&str>,
    model: &str,
    explicit_instructions: Option<&Value>,
    tools: Option<&Value>,
    input: &[Value],
) -> String {
    let system_messages = input
        .iter()
        .filter(|item| {
            item.get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role.eq_ignore_ascii_case("system"))
        })
        .map(responses_input_message_text)
        .collect::<Vec<_>>();
    let seed = json!({
        "tenant_id": tenant_id,
        "continuity_anchor": continuity_anchor,
        "model": model,
        "instructions": {
            "explicit": explicit_instructions.cloned(),
            "system_messages": system_messages,
        },
        "tools": tools.cloned(),
        "gateway_policy": "quality-first-v1",
    });
    format!("tenant:{tenant_id}/prefix:{}", stable_value_digest(&seed))
}

fn cache_continuity_anchor(routing: &RequestRoutingScope) -> Option<&str> {
    match routing.continuity_mode {
        ContinuityMode::CodexWindow | ContinuityMode::SessionAffinity => {
            Some(routing.session_key.as_str())
        }
        ContinuityMode::EphemeralRequest => None,
    }
}

fn stable_value_digest(value: &Value) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(
        serde_json::to_vec(&canonicalize_json(value)).unwrap_or_else(|_| b"stable-value".to_vec()),
    );
    format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5]
    )
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        Value::Object(map) => {
            let mut object = serde_json::Map::new();
            for (key, value) in map.iter().collect::<BTreeMap<_, _>>() {
                object.insert(key.clone(), canonicalize_json(value));
            }
            Value::Object(object)
        }
        _ => value.clone(),
    }
}

fn success_response_summary(
    output_summary: Option<String>,
    response_output_items: &[Value],
    fallback: &str,
) -> String {
    if let Some(output_summary) = output_summary
        .map(|summary| summary.trim().to_string())
        .filter(|summary| !summary.is_empty())
    {
        return truncate_text(output_summary, 240);
    }
    if let Some(output_summary) = summarize_response_output_items(response_output_items) {
        return output_summary;
    }
    fallback.to_string()
}

fn summarize_response_output_items(response_output_items: &[Value]) -> Option<String> {
    let assistant_text = response_output_items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter(|item| item.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(responses_input_message_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !assistant_text.trim().is_empty() {
        return Some(truncate_text(assistant_text, 240));
    }

    let tool_calls = response_output_items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .take(3)
        .map(tool_call_summary)
        .collect::<Vec<_>>();
    if !tool_calls.is_empty() {
        return Some(truncate_text(
            format!("assistant requested tools: {}", tool_calls.join("; ")),
            240,
        ));
    }
    None
}

fn tool_call_summary(item: &Value) -> String {
    let name = item.get("name").and_then(Value::as_str).unwrap_or("tool");
    let arguments = item
        .get("arguments")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match arguments {
        Some(arguments) => format!("{name}({})", truncate_text(arguments, 80)),
        None => format!("{name}()"),
    }
}

fn is_codex_chatgpt_backend(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("/backend-api/codex")
}

fn responses_payload_for_upstream(
    payload: &ResponsesRequest,
    cache_key: String,
    replay_plan: &ReplayPlan,
    codex_protocol: bool,
    codex_input_prefix: &[Value],
) -> Value {
    if codex_protocol {
        return codex_responses_payload(payload, cache_key, replay_plan, codex_input_prefix);
    }

    let mut upstream_payload = payload.clone();
    if upstream_payload.prompt_cache_key.is_none() {
        upstream_payload.prompt_cache_key = Some(cache_key);
    }
    if replay_plan.drop_previous_response_id {
        upstream_payload.extra.remove("previous_response_id");
    }
    if !replay_plan.input_items.is_empty() || replay_plan.fallback_text.is_some() {
        upstream_payload.input = Value::Array(replay_augmented_responses_input(
            &upstream_payload.input,
            replay_plan,
        ));
    }
    serde_json::to_value(&upstream_payload).unwrap_or_else(|_| {
        json!({
            "model": payload.model,
            "input": payload.input,
            "stream": payload.stream.unwrap_or(false)
        })
    })
}

fn replay_augmented_responses_input(input: &Value, replay_plan: &ReplayPlan) -> Vec<Value> {
    prepend_context_messages(
        normalize_responses_input(input.clone()),
        &replay_plan.input_items,
        replay_plan.fallback_text.as_deref(),
    )
    .as_array()
    .cloned()
    .unwrap_or_default()
}

fn compact_retry_request_from_responses(
    payload: &ResponsesRequest,
    replay_plan: &ReplayPlan,
) -> ResponsesRequest {
    let mut compact_request = payload.clone();
    compact_request.input = Value::Array(replay_augmented_responses_input(
        &payload.input,
        replay_plan,
    ));
    compact_request.stream = Some(false);
    compact_request.extra.remove("previous_response_id");
    compact_request
}

fn compact_retry_request_from_chat(
    payload: &ChatCompletionsRequest,
    replay_plan: &ReplayPlan,
) -> ResponsesRequest {
    let input = prepend_context_messages(
        payload
            .messages
            .iter()
            .map(chat_message_to_responses_input)
            .collect::<Vec<_>>(),
        &replay_plan.input_items,
        replay_plan.fallback_text.as_deref(),
    )
    .as_array()
    .cloned()
    .unwrap_or_default();
    ResponsesRequest {
        model: payload.model.clone(),
        input: Value::Array(input),
        stream: Some(false),
        reasoning: payload
            .reasoning_effort
            .as_ref()
            .map(|effort| json!({ "effort": effort })),
        prompt_cache_key: None,
        extra: payload.extra.clone(),
    }
}

async fn compact_retry_output_items(
    state: &AppState,
    credential: &UpstreamCredential,
    lease: &CliLease,
    context: &ForwardContext,
    expected_model: &str,
    compact_request: &ResponsesRequest,
) -> Option<Vec<Value>> {
    let upstream_value = compact_payload_for_upstream(compact_request);
    let fallback_upstream_value = compact_payload_for_standard_responses(compact_request);

    let parse_compacted_output = |value: Value| {
        value
            .get("output")
            .and_then(Value::as_array)
            .cloned()
            .filter(|output| !output.is_empty())
    };

    let parse_response = |value: Value, headers: &reqwest::header::HeaderMap| {
        if let Some(kind) = hidden_failure_kind_from_json(&value, expected_model, headers) {
            tracing::warn!(
                account_id = %lease.account_id,
                route_mode = %lease.route_mode.as_str(),
                hidden_kind = ?kind,
                "transparent compact returned hidden upstream failure"
            );
            return None;
        }
        parse_compacted_output(value)
    };

    match state
        .upstream
        .post_json(
            credential,
            "responses/compact",
            &upstream_value,
            context,
            false,
            lease.route_mode,
        )
        .await
    {
        Ok(response) => {
            let headers = response.headers().clone();
            let value = response.json::<Value>().await.ok()?;
            parse_response(value, &headers)
        }
        Err(error) => {
            tracing::warn!(
                account_id = %lease.account_id,
                route_mode = %lease.route_mode.as_str(),
                status = ?error.status,
                kind = ?error.kind,
                failure_subkind = error.subkind_label(),
                reset_at = ?error.reset_at,
                cf_ray = ?error.cf_ray,
                body_preview = %truncate_text(error.body.clone().unwrap_or_default(), 160),
                "transparent compact request failed"
            );
            if !should_retry_compact_with_standard_responses(&credential.base_url, &error) {
                return None;
            }
            let fallback = state
                .upstream
                .post_json(
                    credential,
                    "responses",
                    &fallback_upstream_value,
                    context,
                    false,
                    lease.route_mode,
                )
                .await
                .ok()?;
            let headers = fallback.headers().clone();
            let value = fallback.json::<Value>().await.ok()?;
            parse_response(value, &headers)
        }
    }
}

fn compact_payload_for_upstream(payload: &ResponsesRequest) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("model".to_string(), Value::String(payload.model.clone()));
    object.insert(
        "input".to_string(),
        Value::Array(normalize_responses_input(payload.input.clone())),
    );

    if let Some(instructions) = payload
        .extra
        .get("instructions")
        .and_then(instruction_text_from_value)
    {
        object.insert("instructions".to_string(), Value::String(instructions));
    }

    let tools = payload
        .extra
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    object.insert("tools".to_string(), Value::Array(tools));

    object.insert(
        "parallel_tool_calls".to_string(),
        Value::Bool(
            payload
                .extra
                .get("parallel_tool_calls")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
    );

    if let Some(reasoning) = payload.reasoning.clone() {
        object.insert("reasoning".to_string(), reasoning);
    }
    if let Some(text) = payload.extra.get("text").cloned() {
        object.insert("text".to_string(), text);
    }

    Value::Object(object)
}

fn compact_payload_for_standard_responses(payload: &ResponsesRequest) -> Value {
    let mut object = compact_payload_for_upstream(payload)
        .as_object()
        .cloned()
        .unwrap_or_default();
    object.insert("stream".to_string(), Value::Bool(false));
    Value::Object(object)
}

fn codex_responses_payload(
    payload: &ResponsesRequest,
    cache_key: String,
    replay_plan: &ReplayPlan,
    codex_input_prefix: &[Value],
) -> Value {
    let mut normalized_input = normalize_responses_input(payload.input.clone());
    if !codex_input_prefix.is_empty() {
        let mut prefixed = codex_input_prefix.to_vec();
        prefixed.extend(normalized_input);
        normalized_input = prefixed;
    }
    let input = prepend_context_messages(
        normalized_input,
        &replay_plan.input_items,
        replay_plan.fallback_text.as_deref(),
    );
    let (instructions, input) =
        codex_instructions_and_input(input, payload.extra.get("instructions"));

    let mut object = serde_json::Map::new();
    object.insert("model".to_string(), Value::String(payload.model.clone()));
    object.insert("instructions".to_string(), Value::String(instructions));
    object.insert("input".to_string(), input);
    object.insert("stream".to_string(), Value::Bool(true));
    object.insert("store".to_string(), Value::Bool(false));
    object.insert(
        "prompt_cache_key".to_string(),
        Value::String(payload.prompt_cache_key.clone().unwrap_or(cache_key)),
    );
    if let Some(reasoning) = payload.reasoning.clone() {
        object.insert("reasoning".to_string(), reasoning);
    }
    for (key, value) in &payload.extra {
        if codex_passthrough_extra_key(key, replay_plan.drop_previous_response_id)
            && !object.contains_key(key)
        {
            object.insert(key.clone(), value.clone());
        }
    }
    let reasoning_requested = object.contains_key("reasoning");
    ensure_reasoning_include_field(&mut object, reasoning_requested);
    Value::Object(object)
}

fn codex_passthrough_extra_key(key: &str, replay_active: bool) -> bool {
    !matches!(
        key,
        // Gateway-owned instructions/streaming fields must stay canonical.
        "instructions" | "store" | "stream" | "prompt_cache_key" | "max_output_tokens"
    ) && !(replay_active && key == "previous_response_id")
}

fn ensure_reasoning_include_field(
    object: &mut serde_json::Map<String, Value>,
    reasoning_requested: bool,
) {
    if !reasoning_requested {
        return;
    }
    let mut include = object
        .remove("include")
        .map(normalize_include_field)
        .unwrap_or_default();
    if !include
        .iter()
        .any(|entry| entry.as_str() == Some("reasoning.encrypted_content"))
    {
        include.push(Value::String("reasoning.encrypted_content".to_string()));
    }
    object.insert("include".to_string(), Value::Array(include));
}

fn normalize_include_field(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::String(item) => vec![Value::String(item)],
        _ => Vec::new(),
    }
}

fn codex_instructions_and_input(
    input: Value,
    explicit_instructions: Option<&Value>,
) -> (String, Value) {
    let mut instructions = Vec::new();
    if let Some(explicit) = explicit_instructions.and_then(instruction_text_from_value) {
        instructions.push(explicit);
    }

    let mut filtered_input = Vec::new();
    for item in normalize_responses_input(input) {
        if item
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| role.eq_ignore_ascii_case("system"))
        {
            let text = responses_input_message_text(&item);
            if !text.is_empty() {
                instructions.push(text);
            }
            continue;
        }
        filtered_input.push(item);
    }

    let instructions = if instructions.is_empty() {
        "You are Codex.".to_string()
    } else {
        instructions.join("\n\n")
    };
    (instructions, Value::Array(filtered_input))
}

fn instruction_text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        other => {
            let text = summarize_value(other);
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
    }
}

fn responses_input_message_text(message: &Value) -> String {
    let Some(content) = message.get("content") else {
        return summarize_value(message);
    };
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.as_str() {
                        return Some(text.to_string());
                    }
                    match item.get("type").and_then(Value::as_str) {
                        Some("input_text") | Some("output_text") | Some("text") => {
                            item.get("text").and_then(Value::as_str).map(str::to_string)
                        }
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() {
                summarize_value(content)
            } else {
                text
            }
        }
        other => summarize_value(other),
    }
}

fn responses_payload_from_chat_request(
    payload: &ChatCompletionsRequest,
    cache_key: String,
    replay_plan: &ReplayPlan,
    codex_protocol: bool,
) -> Value {
    let input = prepend_context_messages(
        payload
            .messages
            .iter()
            .map(chat_message_to_responses_input)
            .collect::<Vec<_>>(),
        &replay_plan.input_items,
        replay_plan.fallback_text.as_deref(),
    )
    .as_array()
    .cloned()
    .unwrap_or_default();
    responses_payload_from_chat_input(payload, input, cache_key, replay_plan, codex_protocol)
}

fn responses_payload_from_chat_input(
    payload: &ChatCompletionsRequest,
    input: Vec<Value>,
    cache_key: String,
    replay_plan: &ReplayPlan,
    codex_protocol: bool,
) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("model".to_string(), Value::String(payload.model.clone()));
    if codex_protocol {
        let (instructions, input) =
            codex_instructions_and_input(Value::Array(input), payload.extra.get("instructions"));
        object.insert("instructions".to_string(), Value::String(instructions));
        object.insert("input".to_string(), input);
        object.insert("stream".to_string(), Value::Bool(true));
        object.insert("store".to_string(), Value::Bool(false));
    } else {
        object.insert("input".to_string(), Value::Array(input));
        object.insert(
            "stream".to_string(),
            Value::Bool(payload.stream.unwrap_or(false)),
        );
    }
    object.insert("prompt_cache_key".to_string(), Value::String(cache_key));
    if let Some(reasoning_effort) = payload.reasoning_effort.as_ref() {
        object.insert(
            "reasoning".to_string(),
            json!({
                "effort": reasoning_effort
            }),
        );
    }
    for (key, value) in &payload.extra {
        if !object.contains_key(key)
            && (!codex_protocol
                || codex_passthrough_extra_key(key, replay_plan.drop_previous_response_id))
        {
            object.insert(key.clone(), value.clone());
        }
    }
    let reasoning_requested = object.contains_key("reasoning");
    ensure_reasoning_include_field(&mut object, reasoning_requested);
    Value::Object(object)
}

fn responses_payload_from_compacted_chat_input(
    payload: &ChatCompletionsRequest,
    input: Vec<Value>,
    cache_key: String,
    codex_protocol: bool,
) -> Value {
    responses_payload_from_chat_input(
        payload,
        input,
        cache_key,
        &ReplayPlan::default(),
        codex_protocol,
    )
}

fn responses_input_function_call_output_call_ids(input: &Value) -> Vec<String> {
    normalize_responses_input(input.clone())
        .into_iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
        .filter_map(|item| {
            item.get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>()
}

fn response_root_value(value: &Value) -> &Value {
    value.get("response").unwrap_or(value)
}

fn response_id_from_value(value: &Value) -> Option<String> {
    response_root_value(value)
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn response_output_items_from_value(value: &Value) -> Vec<Value> {
    response_root_value(value)
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn response_output_item_from_stream_value(value: &Value) -> Option<(usize, Value)> {
    match value.get("type").and_then(Value::as_str) {
        Some("response.output_item.added") | Some("response.output_item.done") => Some((
            value.get("output_index").and_then(Value::as_u64)? as usize,
            value.get("item")?.clone(),
        )),
        _ => None,
    }
}

fn capture_stream_response_metadata(
    value: &Value,
    response_id: &mut Option<String>,
    output_items: &mut BTreeMap<usize, Value>,
    completed_response: &mut Option<Value>,
) {
    if response_id.is_none() {
        *response_id = response_id_from_value(value);
    }
    if let Some((index, item)) = response_output_item_from_stream_value(value) {
        output_items.insert(index, item);
    }
    for (index, item) in response_output_items_from_value(value)
        .into_iter()
        .enumerate()
    {
        output_items.insert(index, item);
    }
    if response_status_is_completed(value) {
        *completed_response = Some(response_root_value(value).clone());
    }
}

fn finalize_stream_response(
    completed_response: Option<Value>,
    response_id: Option<String>,
    output_items: BTreeMap<usize, Value>,
) -> Option<Value> {
    let mut response = completed_response?;
    let Some(object) = response.as_object_mut() else {
        return Some(response);
    };

    if !object.contains_key("id")
        && let Some(response_id) = response_id
    {
        object.insert("id".to_string(), Value::String(response_id));
    }

    let mut merged_output = object
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .collect::<BTreeMap<usize, Value>>();
    for (index, item) in output_items {
        merged_output.insert(index, item);
    }
    if !merged_output.is_empty() {
        object.insert(
            "output".to_string(),
            Value::Array(merged_output.into_values().collect()),
        );
    }

    Some(response)
}

fn response_status_is_completed(value: &Value) -> bool {
    response_root_value(value)
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("completed"))
}

fn normalize_responses_input(input: Value) -> Vec<Value> {
    match input {
        Value::String(text) => vec![json!({
            "role": "user",
            "content": [{"type": "input_text", "text": text}]
        })],
        Value::Array(items) => items,
        Value::Object(map) if map.contains_key("role") && map.contains_key("content") => {
            vec![Value::Object(map)]
        }
        other => vec![json!({
            "role": "user",
            "content": [{"type": "input_text", "text": summarize_value(&other)}]
        })],
    }
}

fn replay_context_message(replay_context: &str) -> Value {
    json!({
        "role": "system",
        "content": [{"type": "input_text", "text": replay_context}]
    })
}

fn chat_message_to_responses_input(message: &ChatMessage) -> Value {
    json!({
        "role": message.role,
        "content": message.content
    })
}

fn responses_json_to_chat_completion(value: &Value, fallback_model: &str) -> Value {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("chatcmpl_proxy");
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(fallback_model);
    let prompt_tokens = usage_value(value, &["input_tokens", "prompt_tokens"]);
    let completion_tokens = usage_value(value, &["output_tokens", "completion_tokens"]);
    let content = extract_response_output_text(value);
    let tool_calls = extract_response_tool_calls(value);
    let finish_reason = response_to_chat_finish_reason(value, !tool_calls.is_empty());
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    message.insert(
        "content".to_string(),
        if content.is_empty() {
            Value::Null
        } else {
            Value::String(content)
        },
    );
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    json!({
        "id": id,
        "object": "chat.completion",
        "created": Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    })
}

fn extract_response_output_text(value: &Value) -> String {
    value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some("assistant"))
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(
                    |content| match content.get("type").and_then(Value::as_str) {
                        Some("output_text") | Some("input_text") => content
                            .get("text")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        _ => None,
                    },
                )
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("")
}

fn extract_response_tool_calls(value: &Value) -> Vec<Value> {
    value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .enumerate()
        .map(|(index, item)| {
            json!({
                "index": index,
                "id": item.get("call_id").and_then(Value::as_str).unwrap_or("call_proxy"),
                "type": "function",
                "function": {
                    "name": item.get("name").and_then(Value::as_str).unwrap_or("tool"),
                    "arguments": item.get("arguments").and_then(Value::as_str).unwrap_or("")
                }
            })
        })
        .collect()
}

fn response_to_chat_finish_reason(value: &Value, saw_tool_call: bool) -> &'static str {
    if saw_tool_call {
        return "tool_calls";
    }
    if response_has_content_filter_terminal(value) {
        return "content_filter";
    }
    if response_has_length_terminal(value) {
        return "length";
    }
    "stop"
}

fn response_has_length_terminal(value: &Value) -> bool {
    if response_error_code(value).is_some_and(|code| is_length_terminal_reason(code)) {
        return true;
    }

    let root = response_root_value(value);
    if root
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("incomplete"))
    {
        if let Some(reason) = response_incomplete_reason(value) {
            return is_length_terminal_reason(reason);
        }
        return true;
    }

    root.get("truncation")
        .and_then(Value::as_str)
        .is_some_and(is_length_terminal_reason)
}

fn response_has_content_filter_terminal(value: &Value) -> bool {
    response_error_code(value).is_some_and(is_content_filter_terminal_reason)
        || response_incomplete_reason(value).is_some_and(is_content_filter_terminal_reason)
}

fn response_incomplete_reason(value: &Value) -> Option<&str> {
    response_root_value(value)
        .get("incomplete_details")
        .and_then(Value::as_object)
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str)
}

fn response_error_code(value: &Value) -> Option<&str> {
    value
        .get("error")
        .and_then(Value::as_object)
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .or_else(|| {
            response_root_value(value)
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
        })
}

fn is_length_terminal_reason(reason: &str) -> bool {
    let normalized = reason.to_ascii_lowercase();
    [
        "length",
        "max_output_tokens",
        "max_tokens",
        "context_length_exceeded",
        "context window",
        "token limit",
        "truncat",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn is_content_filter_terminal_reason(reason: &str) -> bool {
    let normalized = reason.to_ascii_lowercase();
    normalized.contains("content_filter") || normalized.contains("content filter")
}

fn usage_value(value: &Value, keys: &[&str]) -> u64 {
    usage_object(value)
        .and_then(|usage| {
            keys.iter()
                .find_map(|key| usage.get(*key))
                .and_then(Value::as_u64)
        })
        .unwrap_or(0)
}

fn usage_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value
        .get("response")
        .and_then(|response| response.get("usage"))
        .and_then(Value::as_object)
        .or_else(|| value.get("usage").and_then(Value::as_object))
}

#[derive(Debug)]
struct ChatStreamAdapterState {
    chat_id: String,
    model: String,
    created: i64,
    emitted_role: bool,
    saw_text_output: bool,
    saw_tool_call: bool,
    finished: bool,
    tool_calls: Vec<ChatToolCallState>,
}

impl ChatStreamAdapterState {
    fn new(fallback_model: &str, created: i64) -> Self {
        Self {
            chat_id: format!("chatcmpl_{}", uuid::Uuid::new_v4().simple()),
            model: fallback_model.to_string(),
            created,
            emitted_role: false,
            saw_text_output: false,
            saw_tool_call: false,
            finished: false,
            tool_calls: Vec::new(),
        }
    }

    fn ensure_tool_call(
        &mut self,
        output_index: Option<i64>,
        call_id: Option<&str>,
        name: Option<&str>,
    ) -> usize {
        if let Some(call_id) = call_id
            && let Some(index) = self
                .tool_calls
                .iter()
                .position(|tool| tool.call_id == call_id)
        {
            if let Some(output_index) = output_index {
                self.tool_calls[index].output_index = Some(output_index);
            }
            if let Some(name) = name {
                self.tool_calls[index].name = name.to_string();
            }
            return index;
        }

        if let Some(output_index) = output_index
            && let Some(index) = self
                .tool_calls
                .iter()
                .position(|tool| tool.output_index == Some(output_index))
        {
            if let Some(call_id) = call_id {
                self.tool_calls[index].call_id = call_id.to_string();
            }
            if let Some(name) = name {
                self.tool_calls[index].name = name.to_string();
            }
            return index;
        }

        let index = self.tool_calls.len();
        self.tool_calls.push(ChatToolCallState {
            output_index,
            call_id: call_id
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_proxy_{index}")),
            name: name
                .map(str::to_string)
                .unwrap_or_else(|| "tool".to_string()),
            arguments: String::new(),
            emitted_open_chunk: false,
        });
        index
    }

    fn tool_call_mut(
        &mut self,
        output_index: Option<i64>,
        call_id: Option<&str>,
        name: Option<&str>,
    ) -> (usize, &mut ChatToolCallState) {
        let index = self.ensure_tool_call(output_index, call_id, name);
        (index, &mut self.tool_calls[index])
    }
}

#[derive(Debug)]
struct ChatToolCallState {
    output_index: Option<i64>,
    call_id: String,
    name: String,
    arguments: String,
    emitted_open_chunk: bool,
}

fn take_sse_record(buffer: &mut String) -> Option<String> {
    let index = buffer.find("\n\n")?;
    let record = buffer[..index].to_string();
    buffer.drain(..index + 2);
    Some(record)
}

fn translate_response_record_to_chat_events(
    record: &str,
    state: &mut ChatStreamAdapterState,
) -> Vec<Event> {
    let Some((event_name, data)) = parse_sse_record(record) else {
        return Vec::new();
    };
    let value = serde_json::from_str::<Value>(&data).unwrap_or(Value::Null);
    let resolved_event = if event_name.is_empty() {
        value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
    } else {
        event_name.as_str()
    };

    match resolved_event {
        "response.created" => {
            if let Some(id) = value
                .get("response")
                .and_then(|response| response.get("id"))
                .and_then(Value::as_str)
            {
                state.chat_id = id.to_string();
            }
            if let Some(model) = value
                .get("response")
                .and_then(|response| response.get("model"))
                .and_then(Value::as_str)
            {
                state.model = model.to_string();
            }
            if state.emitted_role {
                return Vec::new();
            }
            state.emitted_role = true;
            vec![chat_completion_sse_event(chat_completion_chunk(
                &state.chat_id,
                &state.model,
                state.created,
                json!({"role":"assistant"}),
                None,
            ))]
        }
        "response.output_text.delta" => {
            let delta = value
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut out = Vec::new();
            if !state.emitted_role {
                state.emitted_role = true;
                out.push(chat_completion_sse_event(chat_completion_chunk(
                    &state.chat_id,
                    &state.model,
                    state.created,
                    json!({"role":"assistant"}),
                    None,
                )));
            }
            if !delta.is_empty() {
                state.saw_text_output = true;
                out.push(chat_completion_sse_event(chat_completion_chunk(
                    &state.chat_id,
                    &state.model,
                    state.created,
                    json!({"content": delta}),
                    None,
                )));
            }
            out
        }
        "response.output_item.added" => {
            let Some(item) = value.get("item") else {
                return Vec::new();
            };
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return Vec::new();
            }
            let output_index = value.get("output_index").and_then(Value::as_i64);
            let call_id = item.get("call_id").and_then(Value::as_str);
            let name = item.get("name").and_then(Value::as_str);
            state.saw_tool_call = true;
            let mut out = ensure_assistant_role_event(state);
            let chat_id = state.chat_id.clone();
            let model = state.model.clone();
            let created = state.created;
            let delta = {
                let (tool_index, tool_call) = state.tool_call_mut(output_index, call_id, name);
                if tool_call.emitted_open_chunk {
                    None
                } else {
                    tool_call.emitted_open_chunk = true;
                    Some(json!({
                        "tool_calls": [{
                            "index": tool_index,
                            "id": tool_call.call_id.clone(),
                            "type": "function",
                            "function": {
                                "name": tool_call.name.clone(),
                                "arguments": tool_call.arguments.clone()
                            }
                        }]
                    }))
                }
            };
            if let Some(delta) = delta {
                out.push(chat_completion_sse_event(chat_completion_chunk(
                    &chat_id, &model, created, delta, None,
                )));
            }
            out
        }
        "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
            let output_index = value.get("output_index").and_then(Value::as_i64);
            let call_id = value.get("call_id").and_then(Value::as_str);
            let name = value.get("name").and_then(Value::as_str);
            let arguments = value
                .get("delta")
                .and_then(Value::as_str)
                .or_else(|| value.get("arguments").and_then(Value::as_str))
                .unwrap_or_default();
            state.saw_tool_call = true;
            let mut out = ensure_assistant_role_event(state);
            let chat_id = state.chat_id.clone();
            let model = state.model.clone();
            let created = state.created;
            let (open_delta, emitted_arguments, tool_index) = {
                let (tool_index, tool_call) = state.tool_call_mut(output_index, call_id, name);
                let open_delta = if tool_call.emitted_open_chunk {
                    None
                } else {
                    tool_call.emitted_open_chunk = true;
                    Some(json!({
                        "tool_calls": [{
                            "index": tool_index,
                            "id": tool_call.call_id.clone(),
                            "type": "function",
                            "function": {
                                "name": tool_call.name.clone(),
                                "arguments": ""
                            }
                        }]
                    }))
                };
                let mut emitted_arguments = String::new();
                if !arguments.is_empty() {
                    emitted_arguments = if resolved_event == "response.function_call_arguments.done"
                        && arguments.starts_with(&tool_call.arguments)
                    {
                        arguments[tool_call.arguments.len()..].to_string()
                    } else {
                        arguments.to_string()
                    };
                    if resolved_event == "response.function_call_arguments.done" {
                        tool_call.arguments = arguments.to_string();
                    } else {
                        tool_call.arguments.push_str(arguments);
                    }
                }
                (open_delta, emitted_arguments, tool_index)
            };
            if let Some(open_delta) = open_delta {
                out.push(chat_completion_sse_event(chat_completion_chunk(
                    &chat_id, &model, created, open_delta, None,
                )));
            }
            if !emitted_arguments.is_empty() {
                out.push(chat_completion_sse_event(chat_completion_chunk(
                    &chat_id,
                    &model,
                    created,
                    json!({
                        "tool_calls": [{
                            "index": tool_index,
                            "function": {
                                "arguments": emitted_arguments
                            }
                        }]
                    }),
                    None,
                )));
            }
            out
        }
        "response.output_item.done" => {
            let Some(item) = value.get("item") else {
                return Vec::new();
            };
            match item.get("type").and_then(Value::as_str) {
                Some("function_call") => {
                    let output_index = value.get("output_index").and_then(Value::as_i64);
                    let call_id = item.get("call_id").and_then(Value::as_str);
                    let name = item.get("name").and_then(Value::as_str);
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    state.saw_tool_call = true;
                    let mut out = ensure_assistant_role_event(state);
                    let chat_id = state.chat_id.clone();
                    let model = state.model.clone();
                    let created = state.created;
                    let (open_delta, argument_delta, tool_index) = {
                        let (tool_index, tool_call) =
                            state.tool_call_mut(output_index, call_id, name);
                        let open_delta = if tool_call.emitted_open_chunk {
                            None
                        } else {
                            tool_call.emitted_open_chunk = true;
                            Some(json!({
                                "tool_calls": [{
                                    "index": tool_index,
                                    "id": tool_call.call_id.clone(),
                                    "type": "function",
                                    "function": {
                                        "name": tool_call.name.clone(),
                                        "arguments": ""
                                    }
                                }]
                            }))
                        };
                        let argument_delta = if arguments.is_empty() {
                            None
                        } else if arguments.starts_with(&tool_call.arguments) {
                            let delta = arguments[tool_call.arguments.len()..].to_string();
                            tool_call.arguments = arguments.to_string();
                            (!delta.is_empty()).then_some(delta)
                        } else {
                            tool_call.arguments = arguments.to_string();
                            Some(arguments.to_string())
                        };
                        (open_delta, argument_delta, tool_index)
                    };
                    if let Some(open_delta) = open_delta {
                        out.push(chat_completion_sse_event(chat_completion_chunk(
                            &chat_id, &model, created, open_delta, None,
                        )));
                    }
                    if let Some(argument_delta) = argument_delta {
                        out.push(chat_completion_sse_event(chat_completion_chunk(
                            &chat_id,
                            &model,
                            created,
                            json!({
                                "tool_calls": [{
                                    "index": tool_index,
                                    "function": {
                                        "arguments": argument_delta
                                    }
                                }]
                            }),
                            None,
                        )));
                    }
                    out
                }
                Some("message") if !state.saw_text_output => {
                    let content = item
                        .get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                            Some("output_text") | Some("input_text") => {
                                part.get("text").and_then(Value::as_str)
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    if content.is_empty() {
                        return Vec::new();
                    }
                    state.saw_text_output = true;
                    let mut out = ensure_assistant_role_event(state);
                    out.push(chat_completion_sse_event(chat_completion_chunk(
                        &state.chat_id,
                        &state.model,
                        state.created,
                        json!({"content": content}),
                        None,
                    )));
                    out
                }
                _ => Vec::new(),
            }
        }
        "response.completed" => {
            let mut out = Vec::new();
            let completed_response = value.get("response").unwrap_or(&value);
            if !state.saw_text_output {
                let content = extract_response_output_text(completed_response);
                if !content.is_empty() {
                    state.saw_text_output = true;
                    out.extend(ensure_assistant_role_event(state));
                    out.push(chat_completion_sse_event(chat_completion_chunk(
                        &state.chat_id,
                        &state.model,
                        state.created,
                        json!({"content": content}),
                        None,
                    )));
                }
            }
            state.finished = true;
            out.push(chat_completion_sse_event(chat_completion_chunk(
                &state.chat_id,
                &state.model,
                state.created,
                json!({}),
                Some(response_to_chat_finish_reason(
                    completed_response,
                    state.saw_tool_call,
                )),
            )));
            out.push(Event::default().data("[DONE]"));
            out
        }
        "response.failed" => chat_failure_events_from_response_record(&value, state),
        _ => Vec::new(),
    }
}

fn ensure_assistant_role_event(state: &mut ChatStreamAdapterState) -> Vec<Event> {
    if state.emitted_role {
        return Vec::new();
    }
    state.emitted_role = true;
    vec![chat_completion_sse_event(chat_completion_chunk(
        &state.chat_id,
        &state.model,
        state.created,
        json!({"role":"assistant"}),
        None,
    ))]
}

fn chat_failure_events_from_response_record(
    value: &Value,
    state: &mut ChatStreamAdapterState,
) -> Vec<Event> {
    let response = value.get("response").unwrap_or(value);
    if response_has_content_filter_terminal(response) {
        return chat_terminal_failure_events(state, "content_filter");
    }
    if response_has_length_terminal(response) {
        return chat_terminal_failure_events(state, "length");
    }
    let kind =
        hidden_failure_kind_from_json(value, &state.model, &reqwest::header::HeaderMap::new())
            .unwrap_or(UpstreamFailureKind::Generic);
    chat_gateway_failure_events(state, kind)
}

fn chat_terminal_failure_events(
    state: &mut ChatStreamAdapterState,
    finish_reason: &'static str,
) -> Vec<Event> {
    state.finished = true;
    let mut out = ensure_assistant_role_event(state);
    out.push(chat_completion_sse_event(chat_completion_chunk(
        &state.chat_id,
        &state.model,
        state.created,
        json!({}),
        Some(finish_reason),
    )));
    out.push(Event::default().data("[DONE]"));
    out
}

fn chat_gateway_failure_events(
    state: &mut ChatStreamAdapterState,
    kind: UpstreamFailureKind,
) -> Vec<Event> {
    state.finished = true;
    vec![
        Event::default()
            .event("error")
            .data(gateway_error_value(gateway_failure_reason_from_upstream(kind)).to_string()),
        Event::default().data("[DONE]"),
    ]
}

fn parse_sse_record(record: &str) -> Option<(String, String)> {
    let mut event = String::new();
    let mut data = Vec::new();
    for line in record.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_string());
        }
    }
    if event.is_empty() && data.is_empty() {
        return None;
    }
    Some((event, data.join("\n")))
}

fn chat_completion_chunk(
    id: &str,
    model: &str,
    created: i64,
    delta: Value,
    finish_reason: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason
        }]
    })
}

fn chat_completion_sse_event(value: Value) -> Event {
    Event::default().data(value.to_string())
}

async fn resolve_request_scope(
    state: &AppState,
    auth: &GatewayAuthContext,
    headers: &HeaderMap,
    model: &str,
) -> Result<RequestRoutingScope, ContinuityError> {
    let window_id = nonempty_header_str(headers, "x-codex-window-id");
    if window_id.is_none() && requires_codex_window_id(headers) {
        let error = ContinuityError {
            message: "Codex CLI requests must include x-codex-window-id; refusing to fall back to request-scoped continuity.".to_string(),
        };
        tracing::warn!(
            originator = ?nonempty_header_str(headers, "originator"),
            session_id = ?nonempty_header_str(headers, "session_id"),
            request_id = ?nonempty_header_str(headers, "x-client-request-id"),
            subagent = ?nonempty_header_str(headers, "x-openai-subagent"),
            "rejected codex request without x-codex-window-id"
        );
        return Err(error);
    }

    let Some(window_id) = window_id else {
        if let Some(affinity) = legacy_affinity_key(headers) {
            let principal_id = format!("tenant:{}/principal:{affinity}", auth.tenant.slug);
            return Ok(RequestRoutingScope {
                placement_affinity_key: principal_id.clone(),
                lease_principal_id: principal_id.clone(),
                principal_id,
                session_key: affinity.to_string(),
                window_id: None,
                parent_thread_id: None,
                thread_family_id: None,
                continuity_mode: ContinuityMode::SessionAffinity,
            });
        }

        let request_key = ephemeral_request_key(headers);
        let principal_id = format!("tenant:{}/request:{request_key}", auth.tenant.slug);
        return Ok(RequestRoutingScope {
            placement_affinity_key: format!("tenant:{}/request:{request_key}", auth.tenant.id),
            lease_principal_id: principal_id.clone(),
            principal_id,
            session_key: request_key,
            window_id: None,
            parent_thread_id: None,
            thread_family_id: None,
            continuity_mode: ContinuityMode::EphemeralRequest,
        });
    };

    let parsed_window_id = parse_codex_window_id(window_id)?;
    let parent_thread_id = nonempty_header_str(headers, "x-codex-parent-thread-id")
        .map(normalize_header_thread_id)
        .filter(|value| !value.is_empty());
    let thread = state
        .ensure_conversation_thread(
            &auth.tenant,
            &parsed_window_id.thread_id,
            parent_thread_id.as_deref(),
            Some(model),
            "codex_window",
        )
        .await;
    Ok(RequestRoutingScope {
        principal_id: thread.principal_id.clone(),
        lease_principal_id: format!(
            "tenant:{}/thread-family:{}",
            auth.tenant.slug, thread.root_thread_id
        ),
        placement_affinity_key: format!(
            "tenant:{}/thread-family:{}",
            auth.tenant.id, thread.root_thread_id
        ),
        session_key: thread.root_thread_id.clone(),
        window_id: Some(parsed_window_id.canonical),
        parent_thread_id: thread.parent_thread_id.clone(),
        thread_family_id: Some(thread.root_thread_id.clone()),
        continuity_mode: ContinuityMode::CodexWindow,
    })
}

fn forward_context(headers: &HeaderMap, routing: &RequestRoutingScope) -> ForwardContext {
    let conversation_id = routing.session_key.clone();
    let request_id = nonempty_header_str(headers, "x-client-request-id")
        .unwrap_or(conversation_id.as_str())
        .to_string();
    ForwardContext {
        conversation_id,
        request_id,
        subagent: header_str(headers, "x-openai-subagent").map(str::to_string),
        originator: header_str(headers, "originator").map(str::to_string),
        window_id: routing.window_id.clone(),
        parent_thread_id: routing.parent_thread_id.clone(),
    }
}

fn legacy_affinity_key(headers: &HeaderMap) -> Option<&str> {
    nonempty_header_str(headers, "x-codex-cli-affinity-id")
        .or_else(|| nonempty_header_str(headers, "session_id"))
}

fn requires_codex_window_id(headers: &HeaderMap) -> bool {
    nonempty_header_str(headers, "x-codex-parent-thread-id").is_some()
        || nonempty_header_str(headers, "originator").is_some_and(is_codex_cli_originator)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedCodexWindowId {
    thread_id: String,
    generation: u32,
    canonical: String,
}

fn parse_codex_window_id(value: &str) -> Result<ParsedCodexWindowId, ContinuityError> {
    let trimmed = value.trim();
    let Some((thread_id, generation)) = trimmed.rsplit_once(':') else {
        return Err(ContinuityError {
            message: "x-codex-window-id must use the official <thread_id>:<generation> format."
                .to_string(),
        });
    };
    let thread_id = normalize_header_thread_id(thread_id);
    if thread_id.is_empty() {
        return Err(ContinuityError {
            message: "x-codex-window-id must include a non-empty thread id.".to_string(),
        });
    }
    let generation = generation.parse::<u32>().map_err(|_| ContinuityError {
        message: "x-codex-window-id generation must be an unsigned integer.".to_string(),
    })?;
    Ok(ParsedCodexWindowId {
        canonical: format!("{thread_id}:{generation}"),
        thread_id,
        generation,
    })
}

fn normalize_header_thread_id(value: &str) -> String {
    value.trim().replace(' ', "_")
}

fn is_codex_cli_originator(originator: &str) -> bool {
    originator.eq_ignore_ascii_case(DEFAULT_ORIGINATOR)
        || originator
            .strip_prefix("codex_cli")
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('_'))
}

fn ephemeral_request_key(headers: &HeaderMap) -> String {
    let nonce = Uuid::new_v4().simple().to_string();
    nonempty_header_str(headers, "x-client-request-id")
        .map(|request_id| format!("{request_id}::req:{nonce}"))
        .unwrap_or_else(|| format!("req_{nonce}"))
}

fn replay_turn_count(replay_context: Option<&str>) -> usize {
    replay_context
        .map(|context| {
            if let Some(turns_line) = context.lines().find(|line| line.starts_with("turns=")) {
                return turns_line
                    .trim_start_matches("turns=")
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(0);
            }
            context
                .lines()
                .filter(|line| {
                    line.split_once(". ").is_some_and(|(index, rest)| {
                        index.chars().all(|char| char.is_ascii_digit())
                            && rest.trim_start().starts_with('g')
                    })
                })
                .count()
        })
        .unwrap_or(0)
}

fn log_request_attempt(
    request_log: &RequestLogSeed,
    routing: &RequestRoutingScope,
    context: &ForwardContext,
    lease: &CliLease,
    replay_context: Option<&str>,
    attempt: usize,
) {
    tracing::info!(
        endpoint = request_log.endpoint,
        method = request_log.method,
        attempt = attempt + 1,
        continuity_mode = routing.continuity_mode.as_str(),
        principal_id = %routing.principal_id,
        lease_principal_id = %routing.lease_principal_id,
        window_id = ?routing.window_id,
        thread_family_id = ?routing.thread_family_id,
        parent_thread_id = ?routing.parent_thread_id,
        session_id = %context.conversation_id,
        request_id = %context.request_id,
        subagent = ?context.subagent,
        selected_account_id = %lease.account_id,
        selected_account_label = %lease.account_label,
        generation = lease.generation,
        route_mode = %lease.route_mode.as_str(),
        replay_injected = replay_context.is_some(),
        replay_turns = replay_turn_count(replay_context),
        "forwarding gateway request"
    );
}

fn log_selection_exhausted(
    request_log: &RequestLogSeed,
    routing: &RequestRoutingScope,
    context: &ForwardContext,
    subagent_count: u32,
    exhausted: &LeaseSelectionExhausted,
) {
    tracing::warn!(
        endpoint = request_log.endpoint,
        method = request_log.method,
        requested_model = %request_log.requested_model,
        effective_model = %request_log.effective_model,
        reasoning_effort = ?request_log.reasoning_effort,
        continuity_mode = routing.continuity_mode.as_str(),
        principal_id = %routing.principal_id,
        lease_principal_id = %routing.lease_principal_id,
        window_id = ?routing.window_id,
        thread_family_id = ?routing.thread_family_id,
        parent_thread_id = ?routing.parent_thread_id,
        session_id = %context.conversation_id,
        request_id = %context.request_id,
        subagent = ?context.subagent,
        subagent_count,
        selection_exhausted_kind = exhausted.kind.as_str(),
        attempted_accounts = %exhausted.attempted_account_ids.join(","),
        skipped_quota_count = exhausted.skipped_quota_count,
        skipped_cooldown_count = exhausted.skipped_cooldown_count,
        skipped_inflight_count = exhausted.skipped_inflight_count,
        skipped_capability_count = exhausted.skipped_capability_count,
        skipped_unavailable_count = exhausted.skipped_unavailable_count,
        earliest_retry_at = ?exhausted.earliest_retry_at,
        earliest_reset_at = ?exhausted.earliest_reset_at,
        "gateway lease selection exhausted"
    );
}

fn nonempty_header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    header_str(headers, name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn unauthorized() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": {
                "message": "Missing or invalid gateway key.",
                "type": "unauthorized"
            }
        })),
    )
}

fn summarize_messages(messages: &[ChatMessage]) -> String {
    let joined = messages
        .iter()
        .take(4)
        .map(|message| format!("{}: {}", message.role, summarize_value(&message.content)))
        .collect::<Vec<_>>()
        .join(" | ");
    if joined.is_empty() {
        "empty".to_string()
    } else {
        truncate_text(joined, 220)
    }
}

fn summarize_value(value: &Value) -> String {
    match value {
        Value::Null => "empty".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => truncate_text(text.clone(), 160),
        Value::Array(items) => {
            let summary = items
                .iter()
                .take(4)
                .map(summarize_value)
                .collect::<Vec<_>>()
                .join(" | ");
            if summary.is_empty() {
                "empty".to_string()
            } else {
                truncate_text(summary, 200)
            }
        }
        Value::Object(map) => {
            for key in ["text", "input", "content", "value"] {
                if let Some(value) = map.get(key) {
                    let summary = summarize_value(value);
                    if summary != "empty" {
                        return summary;
                    }
                }
            }
            truncate_text(
                map.iter()
                    .take(5)
                    .map(|(key, value)| format!("{key}={}", summarize_value(value)))
                    .collect::<Vec<_>>()
                    .join(", "),
                200,
            )
        }
    }
}

fn truncate_text(text: impl Into<String>, limit: usize) -> String {
    let text = text.into();
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::{IpAddr, Ipv4Addr},
        sync::Arc,
    };

    use chrono::Utc;
    use http::Request;
    use tokio::sync::{RwLock, mpsc};
    use tower::util::ServiceExt;
    use uuid::Uuid;

    use crate::{
        config::Config,
        models::{
            CacheMetrics, GatewayUserRole, RouteMode, SchedulingSignals, Tenant, UpstreamAccount,
            UpstreamCredential,
        },
        state::RuntimeState,
        upstream::UpstreamClient,
    };

    fn test_state() -> AppState {
        let (writer_tx, _writer_rx) = mpsc::channel(8);
        AppState {
            config: Config {
                bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                data_port: 8080,
                admin_port: 8081,
                max_data_plane_body_bytes: 64 * 1024 * 1024,
                postgres_url: "postgres://localhost/test".to_string(),
                redis_url: "redis://127.0.0.1:6379".to_string(),
                redis_channel: "cmgr:test".to_string(),
                instance_id: "cmgr-test".to_string(),
                browser_assist_url: "http://127.0.0.1:8090".to_string(),
                heartbeat_seconds: 5,
                enable_demo_seed: false,
                account_encryption_key: None,
                direct_proxy_url: None,
                warp_proxy_url: None,
                browser_assist_direct_proxy_url: None,
                browser_assist_warp_proxy_url: None,
            },
            runtime: Arc::new(RuntimeState {
                cache_metrics: RwLock::new(CacheMetrics {
                    cached_tokens: 0,
                    replay_tokens: 0,
                    prefix_hit_ratio: 0.0,
                    request_hit_ratio: 0.0,
                    token_hit_ratio: 0.0,
                    warmup_roi: 0.0,
                    static_prefix_tokens: 0,
                }),
                ..RuntimeState::default()
            }),
            upstream: UpstreamClient::default(),
            writer_tx,
            bus_tx: None,
            persistence: None,
            redis_connected: false,
        }
    }

    fn test_auth_context(tenant: Tenant) -> GatewayAuthContext {
        GatewayAuthContext {
            tenant: tenant.clone(),
            api_key: GatewayApiKey {
                id: Uuid::new_v4(),
                tenant_id: tenant.id,
                name: "test".to_string(),
                email: "test@example.com".to_string(),
                role: GatewayUserRole::Admin,
                token: "token".to_string(),
                default_model: Some("gpt-5.4".to_string()),
                reasoning_effort: None,
                force_model_override: false,
                force_reasoning_effort: false,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        }
    }

    #[tokio::test]
    async fn responses_route_accepts_review_sized_bodies_without_413() {
        let app = router(test_state());
        let oversized_for_axum_default = "x".repeat(3 * 1024 * 1024);
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.4",
            "input": oversized_for_axum_default
        }))
        .expect("serialize request");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn request_usage_reads_cached_tokens_from_prompt_details() {
        let usage = request_usage_from_value(&json!({
            "usage": {
                "input_tokens": 120,
                "output_tokens": 48,
                "total_tokens": 168,
                "prompt_tokens_details": {
                    "cached_tokens": 32
                }
            }
        }));

        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.cached_input_tokens, 32);
        assert_eq!(usage.output_tokens, 48);
        assert_eq!(usage.total_tokens, 168);
    }

    #[tokio::test]
    async fn non_quota_retry_budget_scales_to_matching_account_pool() {
        let state = test_state();
        let tenant_id = Uuid::new_v4();
        let now = Utc::now();
        let base_signals = SchedulingSignals {
            quota_headroom: 0.9,
            quota_headroom_5h: 0.9,
            quota_headroom_7d: 0.9,
            health_score: 0.8,
            egress_stability: 0.8,
            fairness_bias: 0.0,
            inflight: 0,
            capacity: 1,
        };
        {
            let mut accounts = state.runtime.accounts.write().await;
            let mut credentials = state.runtime.credentials.write().await;
            for index in 0..5 {
                let account_id = format!("acct-{index}");
                accounts.insert(
                    account_id.clone(),
                    UpstreamAccount {
                        id: account_id.clone(),
                        tenant_id,
                        label: format!("Account {index}"),
                        models: vec!["gpt-5.4".to_string()],
                        current_mode: RouteMode::Direct,
                        signals: base_signals.clone(),
                        created_at: now,
                    },
                );
                credentials.insert(
                    account_id.clone(),
                    UpstreamCredential {
                        account_id: account_id.clone(),
                        base_url: "http://example.invalid/v1".to_string(),
                        bearer_token: format!("token-{index}"),
                        chatgpt_account_id: None,
                        extra_headers: Vec::new(),
                        managed_auth: None,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            accounts.insert(
                "acct-mismatch".to_string(),
                UpstreamAccount {
                    id: "acct-mismatch".to_string(),
                    tenant_id,
                    label: "Mismatch".to_string(),
                    models: vec!["gpt-4.1".to_string()],
                    current_mode: RouteMode::Direct,
                    signals: base_signals.clone(),
                    created_at: now,
                },
            );
            accounts.insert(
                "acct-other-tenant".to_string(),
                UpstreamAccount {
                    id: "acct-other-tenant".to_string(),
                    tenant_id: Uuid::new_v4(),
                    label: "Other Tenant".to_string(),
                    models: vec!["gpt-5.4".to_string()],
                    current_mode: RouteMode::Direct,
                    signals: base_signals,
                    created_at: now,
                },
            );
        }

        let budget = non_quota_retry_budget(
            &state,
            &LeaseSelectionRequest {
                tenant_id,
                principal_id: "tenant:test/thread:pool".to_string(),
                model: "gpt-5.4".to_string(),
                reasoning_effort: None,
                subagent_count: 0,
                cache_affinity_key: "affinity".to_string(),
                placement_affinity_key: "placement".to_string(),
            },
        )
        .await;

        assert_eq!(budget, 4);
    }

    #[test]
    fn non_quota_retry_budget_keeps_single_account_retry() {
        assert_eq!(non_quota_retry_budget_for_matching_accounts(0), 0);
        assert_eq!(non_quota_retry_budget_for_matching_accounts(1), 1);
        assert_eq!(non_quota_retry_budget_for_matching_accounts(2), 1);
        assert_eq!(non_quota_retry_budget_for_matching_accounts(5), 4);
    }

    #[test]
    fn chat_request_maps_to_responses_payload() {
        let payload = ChatCompletionsRequest {
            model: "gpt-5.4".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: json!("You are exact."),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: json!("hello"),
                },
            ],
            stream: Some(true),
            reasoning_effort: Some("high".to_string()),
            extra: serde_json::Map::from_iter([("tool_choice".to_string(), json!("auto"))]),
        };

        let mapped = responses_payload_from_chat_request(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            false,
        );
        assert_eq!(mapped.get("model").and_then(Value::as_str), Some("gpt-5.4"));
        assert_eq!(mapped.get("stream").and_then(Value::as_bool), Some(true));
        assert_eq!(
            mapped.get("prompt_cache_key").and_then(Value::as_str),
            Some("cache-123")
        );
        assert_eq!(
            mapped
                .get("reasoning")
                .and_then(|reasoning| reasoning.get("effort"))
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            mapped.get("input").and_then(Value::as_array).map(Vec::len),
            Some(2)
        );
        assert_eq!(
            mapped.get("tool_choice").and_then(Value::as_str),
            Some("auto")
        );
    }

    #[test]
    fn replay_context_is_prepended_to_chat_payload() {
        let payload = ChatCompletionsRequest {
            model: "gpt-5.4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: json!("hello"),
            }],
            stream: Some(false),
            reasoning_effort: None,
            extra: serde_json::Map::new(),
        };

        let replay_plan = ReplayPlan {
            input_items: Vec::new(),
            fallback_text: Some("[cmgr replay context]\nrecent_turns=1".to_string()),
            drop_previous_response_id: false,
        };
        let mapped = responses_payload_from_chat_request(
            &payload,
            "cache-123".to_string(),
            &replay_plan,
            false,
        );
        let input = mapped
            .get("input")
            .and_then(Value::as_array)
            .expect("input");
        assert_eq!(input.len(), 2);
        assert_eq!(input[0].get("role").and_then(Value::as_str), Some("system"));
    }

    #[test]
    fn codex_chat_request_extracts_instructions_and_forces_streaming() {
        let payload = ChatCompletionsRequest {
            model: "gpt-5.4".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: json!("Follow the repo policy."),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: json!("hello"),
                },
            ],
            stream: Some(false),
            reasoning_effort: Some("high".to_string()),
            extra: serde_json::Map::new(),
        };

        let mapped = responses_payload_from_chat_request(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            true,
        );
        assert_eq!(mapped.get("stream").and_then(Value::as_bool), Some(true));
        assert_eq!(mapped.get("store").and_then(Value::as_bool), Some(false));
        assert_eq!(
            mapped.get("instructions").and_then(Value::as_str),
            Some("Follow the repo policy.")
        );
        assert_eq!(
            mapped.get("input").and_then(Value::as_array).map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn codex_responses_payload_uses_default_instructions_and_store_false() {
        let payload = ResponsesRequest {
            model: "gpt-5.2".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::new(),
        };

        let mapped = responses_payload_for_upstream(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            true,
            &[],
        );
        assert_eq!(mapped.get("stream").and_then(Value::as_bool), Some(true));
        assert_eq!(mapped.get("store").and_then(Value::as_bool), Some(false));
        assert_eq!(
            mapped.get("instructions").and_then(Value::as_str),
            Some("You are Codex.")
        );
        assert_eq!(
            mapped.get("prompt_cache_key").and_then(Value::as_str),
            Some("cache-123")
        );
    }

    #[test]
    fn codex_responses_payload_drops_unsupported_max_output_tokens() {
        let payload = ResponsesRequest {
            model: "gpt-5.2".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::from_iter([
                ("max_output_tokens".to_string(), json!(16)),
                ("metadata".to_string(), json!({"source": "test"})),
            ]),
        };

        let mapped = responses_payload_for_upstream(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            true,
            &[],
        );
        assert!(mapped.get("max_output_tokens").is_none());
        assert_eq!(
            mapped
                .get("metadata")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("test")
        );
    }

    #[test]
    fn codex_responses_payload_preserves_previous_response_id_without_replay() {
        let payload = ResponsesRequest {
            model: "gpt-5.2".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::from_iter([
                ("previous_response_id".to_string(), json!("resp_prev_123")),
                ("metadata".to_string(), json!({"source": "test"})),
            ]),
        };

        let mapped = responses_payload_for_upstream(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            true,
            &[],
        );
        assert_eq!(
            mapped.get("previous_response_id").and_then(Value::as_str),
            Some("resp_prev_123")
        );
        assert_eq!(
            mapped
                .get("metadata")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("test")
        );
    }

    #[test]
    fn codex_responses_payload_drops_previous_response_id_when_replay_is_injected() {
        let payload = ResponsesRequest {
            model: "gpt-5.2".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::from_iter([(
                "previous_response_id".to_string(),
                json!("resp_prev_123"),
            )]),
        };

        let replay_plan = ReplayPlan {
            input_items: Vec::new(),
            fallback_text: Some(
                "[cmgr replay context]\nrecent_turns=\n1. g1 user: hello".to_string(),
            ),
            drop_previous_response_id: true,
        };
        let mapped = responses_payload_for_upstream(
            &payload,
            "cache-123".to_string(),
            &replay_plan,
            true,
            &[],
        );
        assert!(mapped.get("previous_response_id").is_none());
    }

    #[test]
    fn codex_responses_payload_adds_reasoning_include() {
        let payload = ResponsesRequest {
            model: "gpt-5.2".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: Some(json!({"effort": "high"})),
            prompt_cache_key: None,
            extra: serde_json::Map::from_iter([("include".to_string(), json!(["output_text"]))]),
        };

        let mapped = responses_payload_for_upstream(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            true,
            &[],
        );
        let include = mapped
            .get("include")
            .and_then(Value::as_array)
            .expect("include array");
        assert!(
            include
                .iter()
                .any(|item| item.as_str() == Some("output_text"))
        );
        assert!(
            include
                .iter()
                .any(|item| item.as_str() == Some("reasoning.encrypted_content"))
        );
    }

    #[test]
    fn compact_payload_for_upstream_keeps_only_compact_fields() {
        let payload = ResponsesRequest {
            model: "gpt-5.4".to_string(),
            input: json!({
                "role": "user",
                "content": [{"type": "input_text", "text": "compact me"}]
            }),
            stream: Some(true),
            reasoning: Some(json!({"effort": "high"})),
            prompt_cache_key: Some("cache-123".to_string()),
            extra: serde_json::Map::from_iter([
                (
                    "instructions".to_string(),
                    json!("Summarize the conversation."),
                ),
                (
                    "tools".to_string(),
                    json!([{ "type": "function", "name": "plan" }]),
                ),
                ("parallel_tool_calls".to_string(), json!(true)),
                ("text".to_string(), json!({"verbosity": "low"})),
                ("store".to_string(), json!(false)),
                ("metadata".to_string(), json!({"source": "test"})),
            ]),
        };

        let mapped = compact_payload_for_upstream(&payload);
        assert_eq!(mapped.get("model").and_then(Value::as_str), Some("gpt-5.4"));
        assert_eq!(
            mapped.get("instructions").and_then(Value::as_str),
            Some("Summarize the conversation.")
        );
        assert_eq!(
            mapped.get("input").and_then(Value::as_array).map(Vec::len),
            Some(1)
        );
        assert_eq!(
            mapped.get("tools").and_then(Value::as_array).map(Vec::len),
            Some(1)
        );
        assert_eq!(
            mapped.get("parallel_tool_calls").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            mapped
                .get("reasoning")
                .and_then(|reasoning| reasoning.get("effort"))
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            mapped
                .get("text")
                .and_then(|text| text.get("verbosity"))
                .and_then(Value::as_str),
            Some("low")
        );
        assert!(mapped.get("stream").is_none());
        assert!(mapped.get("store").is_none());
        assert!(mapped.get("prompt_cache_key").is_none());
        assert!(mapped.get("metadata").is_none());
    }

    #[test]
    fn compact_payload_for_upstream_defaults_tools_and_parallel_tool_calls() {
        let payload = ResponsesRequest {
            model: "gpt-5.4".to_string(),
            input: json!("compact me"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::new(),
        };

        let mapped = compact_payload_for_upstream(&payload);
        assert_eq!(
            mapped.get("input").and_then(Value::as_array).map(Vec::len),
            Some(1)
        );
        assert_eq!(
            mapped.get("tools").and_then(Value::as_array).map(Vec::len),
            Some(0)
        );
        assert_eq!(
            mapped.get("parallel_tool_calls").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn compact_payload_for_standard_responses_adds_non_stream_flag() {
        let payload = ResponsesRequest {
            model: "gpt-5.4".to_string(),
            input: json!("compact me"),
            stream: Some(true),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::new(),
        };

        let mapped = compact_payload_for_standard_responses(&payload);
        assert_eq!(mapped.get("stream").and_then(Value::as_bool), Some(false));
        assert_eq!(mapped.get("model").and_then(Value::as_str), Some("gpt-5.4"));
    }

    #[test]
    fn passthrough_compact_error_detects_context_window_failures() {
        let error = UpstreamFailure {
            status: Some(StatusCode::BAD_REQUEST),
            body: Some(
                json!({
                    "error": {
                        "code": "context_length_exceeded",
                        "message": "Your input exceeds the context window of this model."
                    }
                })
                .to_string(),
            ),
            kind: UpstreamFailureKind::Generic,
            subkind: None,
            cf_ray: None,
            reset_at: None,
        };

        assert!(should_passthrough_compact_upstream_error(&error));
    }

    #[test]
    fn compact_standard_responses_retry_only_applies_to_chatgpt_codex_backends() {
        let generic_error = UpstreamFailure {
            status: Some(StatusCode::SERVICE_UNAVAILABLE),
            body: Some("temporary upstream failure".to_string()),
            kind: UpstreamFailureKind::Generic,
            subkind: None,
            cf_ray: None,
            reset_at: None,
        };

        assert!(should_retry_compact_with_standard_responses(
            "https://chatgpt.com/backend-api/codex",
            &generic_error
        ));
        assert!(!should_retry_compact_with_standard_responses(
            "https://api.openai.com/v1",
            &generic_error
        ));
    }

    #[test]
    fn compact_retry_request_from_responses_replays_prior_window_and_drops_previous_response_id() {
        let payload = ResponsesRequest {
            model: "gpt-5.4".to_string(),
            input: json!([{
                "role": "user",
                "content": [{"type": "input_text", "text": "new turn"}]
            }]),
            stream: Some(true),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::from_iter([(
                "previous_response_id".to_string(),
                json!("resp_prev_123"),
            )]),
        };
        let replay_plan = ReplayPlan {
            input_items: vec![json!({
                "role": "assistant",
                "content": [{"type": "output_text", "text": "prior answer"}]
            })],
            fallback_text: Some("[cmgr replay context]\nrecent_turns=\n1. prior".to_string()),
            drop_previous_response_id: true,
        };

        let compacted = compact_retry_request_from_responses(&payload, &replay_plan);
        let input = compacted
            .input
            .as_array()
            .cloned()
            .expect("compacted input array");

        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "assistant");
        assert_eq!(input[1]["role"], "system");
        assert_eq!(input[2]["role"], "user");
        assert!(compacted.extra.get("previous_response_id").is_none());
        assert_eq!(compacted.stream, Some(false));
    }

    #[test]
    fn compact_retry_request_from_chat_preserves_current_turn_and_replay_items() {
        let payload = ChatCompletionsRequest {
            model: "gpt-5.4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: json!([{"type": "text", "text": "latest question"}]),
            }],
            stream: Some(true),
            reasoning_effort: Some("medium".to_string()),
            extra: serde_json::Map::from_iter([(
                "previous_response_id".to_string(),
                json!("resp_prev_123"),
            )]),
        };
        let replay_plan = ReplayPlan {
            input_items: vec![json!({
                "role": "assistant",
                "content": [{"type": "output_text", "text": "previous answer"}]
            })],
            fallback_text: None,
            drop_previous_response_id: false,
        };

        let compacted = compact_retry_request_from_chat(&payload, &replay_plan);
        let input = compacted
            .input
            .as_array()
            .cloned()
            .expect("compacted input array");

        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["role"], "assistant");
        assert_eq!(input[1]["role"], "user");
        assert_eq!(
            compacted
                .reasoning
                .as_ref()
                .and_then(|reasoning| reasoning.get("effort"))
                .and_then(Value::as_str),
            Some("medium")
        );
    }

    #[test]
    fn non_codex_responses_payload_preserves_previous_response_id() {
        let payload = ResponsesRequest {
            model: "gpt-5.2".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::from_iter([(
                "previous_response_id".to_string(),
                json!("resp_prev_123"),
            )]),
        };

        let mapped = responses_payload_for_upstream(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            false,
            &[],
        );
        assert_eq!(
            mapped.get("previous_response_id").and_then(Value::as_str),
            Some("resp_prev_123")
        );
    }

    #[test]
    fn codex_responses_payload_prepends_replayed_function_calls() {
        let payload = ResponsesRequest {
            model: "gpt-5.2".to_string(),
            input: json!([{
                "type": "function_call_output",
                "call_id": "call_plan_123",
                "output": "Plan updated"
            }]),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::new(),
        };

        let mapped = responses_payload_for_upstream(
            &payload,
            "cache-123".to_string(),
            &ReplayPlan::default(),
            true,
            &[json!({
                "type": "function_call",
                "call_id": "call_plan_123",
                "name": "update_plan",
                "arguments": "{\"plan\":[{\"step\":\"Inspect logs\",\"status\":\"completed\"}]}"
            })],
        );
        let input = mapped
            .get("input")
            .and_then(Value::as_array)
            .expect("input");

        assert_eq!(input.len(), 2);
        assert_eq!(
            input[0].get("type").and_then(Value::as_str),
            Some("function_call")
        );
        assert_eq!(
            input[0].get("call_id").and_then(Value::as_str),
            Some("call_plan_123")
        );
        assert_eq!(
            input[1].get("type").and_then(Value::as_str),
            Some("function_call_output")
        );
    }

    #[test]
    fn parse_responses_ws_create_strips_transport_fields() {
        let payload = parse_responses_ws_create(
            r#"{"type":"response.create","model":"gpt-5.4","input":"hello","background":true,"metadata":{"source":"test"}}"#,
        )
        .expect("parse websocket response.create");

        assert_eq!(payload.model, "gpt-5.4");
        assert_eq!(payload.input, json!("hello"));
        assert!(payload.extra.get("background").is_none());
        assert_eq!(
            payload
                .extra
                .get("metadata")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("test")
        );
    }

    #[test]
    fn extract_ws_generate_defaults_true_and_strips_field() {
        let mut payload = parse_responses_ws_create(
            r#"{"type":"response.create","model":"gpt-5.4","input":"hello","generate":true}"#,
        )
        .expect("parse websocket response.create");

        assert_eq!(extract_ws_generate(&mut payload), Ok(true));
        assert!(payload.extra.get("generate").is_none());
    }

    #[test]
    fn extract_ws_generate_rejects_non_boolean_values() {
        let mut payload = parse_responses_ws_create(
            r#"{"type":"response.create","model":"gpt-5.4","input":"hello","generate":"yes"}"#,
        )
        .expect("parse websocket response.create");

        assert_eq!(
            extract_ws_generate(&mut payload),
            Err("invalid response.create payload: generate must be a boolean".to_string())
        );
    }

    #[test]
    fn websocket_failure_response_event_emits_failed_status() {
        let event = websocket_failure_response_event(GatewayFailureReason::Queue);
        assert_eq!(
            event.get("type").and_then(Value::as_str),
            Some("response.failed")
        );
        assert_eq!(event["response"]["status"].as_str(), Some("failed"));
        assert_eq!(event["response"]["error"]["reason"].as_str(), Some("queue"));
    }

    #[test]
    fn selection_wait_interval_prefers_retry_time() {
        let retry_at = Utc::now() + chrono::Duration::seconds(30);
        let reset_at = Utc::now() + chrono::Duration::seconds(120);

        let wait = selection_wait_interval(Some(retry_at), Some(reset_at));

        assert!(wait <= Duration::from_secs(5));
    }

    #[test]
    fn resolve_effective_model_maps_codex_worker_model_to_default() {
        let api_key = GatewayApiKey {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            name: "test".to_string(),
            email: "test@example.com".to_string(),
            role: GatewayUserRole::Admin,
            token: "token".to_string(),
            default_model: Some("gpt-5.4".to_string()),
            reasoning_effort: None,
            force_model_override: false,
            force_reasoning_effort: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(
            resolve_effective_model(&api_key, "gpt-5.1-codex-mini"),
            "gpt-5.4"
        );
    }

    #[test]
    fn resolve_effective_model_keeps_supported_requested_model() {
        let api_key = GatewayApiKey {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            name: "test".to_string(),
            email: "test@example.com".to_string(),
            role: GatewayUserRole::Admin,
            token: "token".to_string(),
            default_model: Some("gpt-5.4".to_string()),
            reasoning_effort: None,
            force_model_override: false,
            force_reasoning_effort: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(resolve_effective_model(&api_key, "gpt-5.4"), "gpt-5.4");
    }

    #[test]
    fn resolve_effective_model_maps_gpt5_family_alias_to_default() {
        let api_key = GatewayApiKey {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            name: "test".to_string(),
            email: "test@example.com".to_string(),
            role: GatewayUserRole::Admin,
            token: "token".to_string(),
            default_model: Some("gpt-5.4".to_string()),
            reasoning_effort: None,
            force_model_override: false,
            force_reasoning_effort: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(resolve_effective_model(&api_key, "gpt-5"), "gpt-5.4");
        assert_eq!(resolve_effective_model(&api_key, "gpt-5.1"), "gpt-5.4");
        assert_eq!(resolve_effective_model(&api_key, "gpt-5.1-high"), "gpt-5.4");
    }

    #[test]
    fn resolve_effective_model_maps_codex_family_alias_to_supported_codex_model() {
        let api_key = GatewayApiKey {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            name: "test".to_string(),
            email: "test@example.com".to_string(),
            role: GatewayUserRole::Admin,
            token: "token".to_string(),
            default_model: Some("gpt-5.4".to_string()),
            reasoning_effort: None,
            force_model_override: false,
            force_reasoning_effort: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(
            resolve_effective_model(&api_key, "gpt-5.1-codex"),
            "gpt-5.3-codex"
        );
        assert_eq!(
            resolve_effective_model(&api_key, "gpt-5.1-codex-max"),
            "gpt-5.3-codex"
        );
    }

    #[test]
    fn codex_worker_model_fallback_uses_codex_default_when_missing_api_key_default() {
        assert_eq!(
            codex_worker_model_fallback("gpt-5.1-codex-mini", None).as_deref(),
            Some("gpt-5.3-codex")
        );
    }

    #[test]
    fn legacy_affinity_key_ignores_request_id_only_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-client-request-id", HeaderValue::from_static("req-123"));
        assert!(legacy_affinity_key(&headers).is_none());
        assert!(ephemeral_request_key(&headers).starts_with("req-123::req:"));
    }

    #[test]
    fn legacy_affinity_key_ignores_subagent_only_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-openai-subagent", HeaderValue::from_static("worker-2"));
        assert!(legacy_affinity_key(&headers).is_none());
    }

    #[test]
    fn codex_cli_originator_requires_window_id() {
        let mut headers = HeaderMap::new();
        headers.insert("originator", HeaderValue::from_static(DEFAULT_ORIGINATOR));
        assert!(requires_codex_window_id(&headers));
    }

    #[test]
    fn affinity_only_requests_do_not_require_window_id() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-cli-affinity-id",
            HeaderValue::from_static("cli-affinity"),
        );
        assert!(!requires_codex_window_id(&headers));
    }

    #[test]
    fn forward_context_uses_stable_session_key_before_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert("x-client-request-id", HeaderValue::from_static("req-123"));
        let routing = RequestRoutingScope {
            principal_id: "tenant:test/principal:thread-1".to_string(),
            lease_principal_id: "tenant:test/thread-family:root-1".to_string(),
            placement_affinity_key: "tenant:test/thread-family:root-1".to_string(),
            session_key: "root-1".to_string(),
            window_id: Some("thread-1".to_string()),
            parent_thread_id: Some("parent-1".to_string()),
            thread_family_id: Some("root-1".to_string()),
            continuity_mode: ContinuityMode::CodexWindow,
        };

        let context = forward_context(&headers, &routing);
        assert_eq!(context.conversation_id, "root-1");
        assert_eq!(context.request_id, "req-123");
        assert_eq!(context.window_id.as_deref(), Some("thread-1"));
    }

    #[tokio::test]
    async fn codex_window_routing_uses_root_thread_as_session_key() {
        let state = test_state();
        let tenant = Tenant {
            id: Uuid::new_v4(),
            slug: "test".to_string(),
            name: "Test".to_string(),
            created_at: Utc::now(),
        };
        let auth = test_auth_context(tenant);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-codex-window-id",
            HeaderValue::from_static("child-thread:3"),
        );
        headers.insert(
            "x-codex-parent-thread-id",
            HeaderValue::from_static("root-thread"),
        );

        let routing = resolve_request_scope(&state, &auth, &headers, "gpt-5.4")
            .await
            .expect("routing");
        let context = forward_context(&headers, &routing);

        assert_eq!(routing.session_key, "root-thread");
        assert_eq!(
            routing.lease_principal_id,
            "tenant:test/thread-family:root-thread"
        );
        assert_eq!(routing.thread_family_id.as_deref(), Some("root-thread"));
        assert_eq!(context.conversation_id, "root-thread");
        assert_eq!(context.window_id.as_deref(), Some("child-thread:3"));
    }

    #[test]
    fn replay_turn_count_counts_recent_turn_lines() {
        let replay = "[cmgr replay context]\nrecent_turns=\n1. g1 user: hello\n   assistant: ok\n2. g2 user: ship it\n";
        assert_eq!(replay_turn_count(Some(replay)), 2);
        assert_eq!(replay_turn_count(None), 0);
    }

    #[test]
    fn cache_affinity_key_is_stable_without_legacy_behavior_profile() {
        let payload = ResponsesRequest {
            model: "gpt-5.4".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::new(),
        };

        assert_eq!(
            responses_cache_affinity_key(Uuid::nil(), Some("thread-1"), &payload),
            responses_cache_affinity_key(Uuid::nil(), Some("thread-1"), &payload)
        );
    }

    #[test]
    fn cache_affinity_key_changes_across_continuity_anchors() {
        let payload = ResponsesRequest {
            model: "gpt-5.4".to_string(),
            input: json!("hello"),
            stream: Some(false),
            reasoning: None,
            prompt_cache_key: None,
            extra: serde_json::Map::new(),
        };

        assert_ne!(
            responses_cache_affinity_key(Uuid::nil(), Some("thread-root"), &payload,),
            responses_cache_affinity_key(Uuid::nil(), Some("thread-child"), &payload,)
        );
    }

    #[test]
    fn parse_codex_window_id_extracts_thread_and_generation() {
        let parsed = parse_codex_window_id("thread-123:7").expect("parse window id");
        assert_eq!(parsed.thread_id, "thread-123");
        assert_eq!(parsed.generation, 7);
        assert_eq!(parsed.canonical, "thread-123:7");
    }

    #[test]
    fn success_response_summary_uses_tool_call_details() {
        let summary = success_response_summary(
            None,
            &[json!({
                "type": "function_call",
                "call_id": "call_shell_1",
                "name": "shell",
                "arguments": "{\"command\":\"echo hi\"}"
            })],
            "fallback",
        );
        assert!(summary.contains("shell"));
        assert_ne!(summary, "assistant tool response delivered");
    }

    #[test]
    fn responses_json_maps_to_chat_completion() {
        let value = json!({
            "id": "resp_123",
            "model": "gpt-5.4",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": "hello "},
                    {"type": "output_text", "text": "world"}
                ]
            }],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 7
            }
        });

        let mapped = responses_json_to_chat_completion(&value, "fallback");
        assert_eq!(mapped.get("id").and_then(Value::as_str), Some("resp_123"));
        assert_eq!(mapped.get("model").and_then(Value::as_str), Some("gpt-5.4"));
        assert_eq!(
            mapped
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some("hello world")
        );
        assert_eq!(
            mapped
                .get("usage")
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64),
            Some(19)
        );
    }

    #[test]
    fn responses_json_maps_tool_calls_to_chat_completion() {
        let value = json!({
            "id": "resp_tool_123",
            "model": "gpt-5.4",
            "output": [{
                "type": "function_call",
                "call_id": "call_shell_1",
                "name": "shell",
                "arguments": "{\"command\":\"echo hi\"}"
            }],
            "usage": {
                "input_tokens": 11,
                "output_tokens": 3
            }
        });

        let mapped = responses_json_to_chat_completion(&value, "fallback");
        assert_eq!(
            mapped["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "shell"
        );
        assert_eq!(
            mapped["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{\"command\":\"echo hi\"}"
        );
        assert_eq!(mapped["choices"][0]["message"]["content"], Value::Null);
        assert_eq!(mapped["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn response_terminal_reason_maps_incomplete_to_length() {
        let value = json!({
            "status": "incomplete",
            "incomplete_details": {
                "reason": "max_output_tokens"
            }
        });

        assert_eq!(response_to_chat_finish_reason(&value, false), "length");
    }

    #[test]
    fn response_terminal_reason_maps_context_error_to_length() {
        let value = json!({
            "status": "failed",
            "error": {
                "code": "context_length_exceeded"
            }
        });

        assert_eq!(response_to_chat_finish_reason(&value, false), "length");
    }

    #[test]
    fn response_terminal_reason_maps_content_filter() {
        let value = json!({
            "status": "incomplete",
            "incomplete_details": {
                "reason": "content_filter"
            }
        });

        assert_eq!(
            response_to_chat_finish_reason(&value, false),
            "content_filter"
        );
    }

    #[test]
    fn response_sse_events_translate_to_chat_chunks() {
        let created = Utc::now().timestamp();
        let mut state = ChatStreamAdapterState::new("gpt-5.4", created);

        let created_events = translate_response_record_to_chat_events(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"model\":\"gpt-5.4\"}}",
            &mut state,
        );
        assert_eq!(created_events.len(), 1);

        let delta_events = translate_response_record_to_chat_events(
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}",
            &mut state,
        );
        assert_eq!(delta_events.len(), 1);

        let completed_events = translate_response_record_to_chat_events(
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\"}}",
            &mut state,
        );
        assert_eq!(completed_events.len(), 2);
        assert!(state.finished);
    }

    #[test]
    fn response_sse_tool_events_translate_to_chat_chunks() {
        let created = Utc::now().timestamp();
        let mut state = ChatStreamAdapterState::new("gpt-5.4", created);

        let created_events = translate_response_record_to_chat_events(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_tool_1\",\"model\":\"gpt-5.4\"}}",
            &mut state,
        );
        assert_eq!(created_events.len(), 1);

        let added_events = translate_response_record_to_chat_events(
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"shell\"}}",
            &mut state,
        );
        assert_eq!(added_events.len(), 1);
        assert!(state.saw_tool_call);
        assert_eq!(state.tool_calls.len(), 1);
        assert_eq!(state.tool_calls[0].name, "shell");

        let delta_events = translate_response_record_to_chat_events(
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"command\\\":\\\"echo hi\\\"}\"}",
            &mut state,
        );
        assert_eq!(delta_events.len(), 1);
        assert_eq!(state.tool_calls[0].arguments, "{\"command\":\"echo hi\"}");

        let completed_events = translate_response_record_to_chat_events(
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_tool_1\",\"status\":\"completed\"}}",
            &mut state,
        );
        assert_eq!(completed_events.len(), 2);
        assert!(state.finished);
    }

    #[test]
    fn response_failed_sse_event_terminates_chat_stream() {
        let created = Utc::now().timestamp();
        let mut state = ChatStreamAdapterState::new("gpt-5.4", created);

        let created_events = translate_response_record_to_chat_events(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_fail_1\",\"model\":\"gpt-5.4\"}}",
            &mut state,
        );
        assert_eq!(created_events.len(), 1);

        let failed_events = translate_response_record_to_chat_events(
            "event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_fail_1\",\"status\":\"failed\",\"error\":{\"code\":\"context_length_exceeded\",\"message\":\"context window exceeded\"}}}",
            &mut state,
        );
        assert_eq!(failed_events.len(), 2);
        assert!(state.finished);
    }

    #[test]
    fn finalize_stream_response_preserves_streamed_function_calls() {
        let mut response_id = None;
        let mut output_items = BTreeMap::new();
        let mut completed_response = None;

        capture_stream_response_metadata(
            &json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "update_plan",
                    "arguments": "{\"plan\":[]}"
                }
            }),
            &mut response_id,
            &mut output_items,
            &mut completed_response,
        );
        capture_stream_response_metadata(
            &json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "status": "completed"
                }
            }),
            &mut response_id,
            &mut output_items,
            &mut completed_response,
        );

        let finalized = finalize_stream_response(completed_response, response_id, output_items)
            .expect("finalized response");

        assert_eq!(finalized.get("id").and_then(Value::as_str), Some("resp_1"));
        assert_eq!(
            finalized
                .get("output")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("function_call")
        );
        assert_eq!(
            finalized
                .get("output")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("call_id"))
                .and_then(Value::as_str),
            Some("call_1")
        );
    }

    #[test]
    fn hidden_failure_detects_quota_failure_payload() {
        let payload = json!({
            "type": "response.failed",
            "response": {
                "id": "resp_quota",
                "status": "failed",
                "error": {
                    "code": "insufficient_quota",
                    "message": "You exceeded your current quota."
                }
            }
        });

        let kind =
            hidden_failure_kind_from_json(&payload, "gpt-5.4", &reqwest::header::HeaderMap::new());
        assert_eq!(kind, Some(UpstreamFailureKind::Quota));
    }

    #[test]
    fn hidden_failure_detects_context_window_payload() {
        let payload = json!({
            "type": "response.failed",
            "response": {
                "id": "resp_length",
                "status": "failed",
                "error": {
                    "code": "context_length_exceeded",
                    "message": "The request exceeds the model context window."
                }
            }
        });

        let kind =
            hidden_failure_kind_from_json(&payload, "gpt-5.4", &reqwest::header::HeaderMap::new());
        assert_eq!(kind, Some(UpstreamFailureKind::Length));
    }

    #[test]
    fn hidden_failure_detects_model_drift() {
        let payload = json!({
            "id": "resp_drift",
            "status": "completed",
            "model": "gpt-4.1-mini"
        });

        let kind =
            hidden_failure_kind_from_json(&payload, "gpt-5.4", &reqwest::header::HeaderMap::new());
        assert_eq!(kind, Some(UpstreamFailureKind::Capability));
    }

    #[test]
    fn versioned_model_alias_does_not_count_as_drift() {
        let payload = json!({
            "id": "resp_alias",
            "status": "completed",
            "model": "gpt-5.2-2025-12-11"
        });

        let kind =
            hidden_failure_kind_from_json(&payload, "gpt-5.2", &reqwest::header::HeaderMap::new());
        assert_eq!(kind, None);
        assert!(model_matches_expected(
            "gpt-5.3-codex-2025-12-11",
            "gpt-5.3-codex"
        ));
    }

    #[test]
    fn hidden_failure_detects_sse_response_failed() {
        let record = "event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"Rate limit reached\"}}}";
        assert_eq!(
            hidden_failure_kind_from_sse_record(record, "gpt-5.4"),
            Some(UpstreamFailureKind::Quota)
        );
    }
}
