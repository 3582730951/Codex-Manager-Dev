use std::{convert::Infallible, pin::Pin, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, HeaderValue, Response, StatusCode, header::CONTENT_TYPE},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::Utc;
use futures_util::{StreamExt, TryStreamExt};
use serde_json::{Value, json};

use crate::{
    models::{
        ChatCompletionsRequest, ChatMessage, CliLease, LeaseSelectionRequest, ResponsesRequest,
        RouteEventRequest,
    },
    state::AppState,
    upstream::{ForwardContext, UpstreamFailureKind, classify_failure_body},
};

enum ForwardOutcome {
    Response(Response<Body>, Option<String>),
    HiddenFailure(UpstreamFailureKind),
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/responses", post(responses))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({
        "service": "server-data",
        "status": "ok"
    }))
}

async fn models(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    match authenticated_tenant(&state, &headers).await {
        Some(tenant) => {
            let accounts = state.runtime.accounts.read().await;
            let mut items = accounts
                .values()
                .filter(|account| account.tenant_id == tenant.id)
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

async fn responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ResponsesRequest>,
) -> Response<Body> {
    let Some(tenant) = authenticated_tenant(&state, &headers).await else {
        return unauthorized().into_response();
    };
    let principal_id = derive_principal_id(&headers, tenant.slug.as_str());
    let subagent_count = headers.get("x-openai-subagent").map(|_| 1_u32).unwrap_or(0);
    let model = payload.model.clone();
    let input_summary = summarize_value(&payload.input);
    let selection_request = LeaseSelectionRequest {
        tenant_id: tenant.id,
        principal_id: principal_id.clone(),
        model: model.clone(),
        reasoning_effort: None,
        subagent_count,
    };
    let mut selection = state.resolve_lease(selection_request.clone()).await;
    let stream_requested = payload.stream.unwrap_or(false);
    let mut recorded_input = false;
    let context = forward_context(&headers, &principal_id);

    for attempt in 0..2 {
        let Some((lease, replay, _warmup)) = selection.take() else {
            return waiting_response(stream_requested, state.config.heartbeat_seconds);
        };
        if !recorded_input {
            state
                .record_context_input(
                    &principal_id,
                    &model,
                    lease.generation,
                    input_summary.clone(),
                )
                .await;
            recorded_input = true;
        }
        let replay_context = state
            .replay_context_for(&principal_id, lease.generation)
            .await;
        let Some(credential) = state.credential_for_account(&lease.account_id).await else {
            let _ = state
                .failover_account(&lease.account_id, "credential-missing", 300, true)
                .await;
            tracing::warn!(
                account_id = %lease.account_id,
                principal_id = %principal_id,
                "selected account missing credential, retrying hidden failover"
            );
            if attempt == 0 {
                selection = state.resolve_lease(selection_request.clone()).await;
                continue;
            }
            return waiting_response(stream_requested, state.config.heartbeat_seconds);
        };

        let codex_protocol = is_codex_chatgpt_backend(&credential.base_url);
        let upstream_value = responses_payload_for_upstream(
            &payload,
            replay.cache_key.clone(),
            replay_context.as_deref(),
            codex_protocol,
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
                            principal_id.clone(),
                            &model,
                            state.config.heartbeat_seconds,
                        )
                        .await
                    } else {
                        passthrough_stream_response(
                            response,
                            state.clone(),
                            lease.clone(),
                            principal_id.clone(),
                            &model,
                        )
                    } {
                        ForwardOutcome::Response(response, _) => return response,
                        ForwardOutcome::HiddenFailure(kind) => {
                            handle_hidden_failure(&state, &lease, kind).await;
                            if attempt == 0 && kind.requires_failover() {
                                selection = state.resolve_lease(selection_request.clone()).await;
                                continue;
                            }
                            return waiting_response(
                                stream_requested,
                                state.config.heartbeat_seconds,
                            );
                        }
                    }
                }
                match if codex_protocol {
                    upstream_stream_to_json_response(response, &model).await
                } else {
                    upstream_json_response(response, &model).await
                } {
                    ForwardOutcome::Response(response, output_summary) => {
                        let _ = state
                            .record_route_event(
                                &lease.account_id,
                                RouteEventRequest {
                                    mode: lease.route_mode,
                                    kind: "success".to_string(),
                                },
                            )
                            .await;
                        if let Some(output_summary) = output_summary {
                            state
                                .record_context_output(&principal_id, output_summary)
                                .await;
                        }
                        return response;
                    }
                    ForwardOutcome::HiddenFailure(kind) => {
                        handle_hidden_failure(&state, &lease, kind).await;
                        if attempt == 0 && kind.requires_failover() {
                            selection = state.resolve_lease(selection_request.clone()).await;
                            continue;
                        }
                        return waiting_response(stream_requested, state.config.heartbeat_seconds);
                    }
                }
            }
            Err(error) => {
                handle_hidden_failure(&state, &lease, error.kind).await;
                tracing::warn!(
                    account_id = %lease.account_id,
                    route_mode = %lease.route_mode.as_str(),
                    status = ?error.status,
                    kind = ?error.kind,
                    body_preview = %truncate_text(error.body.unwrap_or_default(), 160),
                    "responses upstream request failed"
                );
                if attempt == 0 && error.kind.requires_failover() {
                    selection = state.resolve_lease(selection_request.clone()).await;
                    continue;
                }
                return waiting_response(stream_requested, state.config.heartbeat_seconds);
            }
        }
    }

    waiting_response(stream_requested, state.config.heartbeat_seconds)
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChatCompletionsRequest>,
) -> Response<Body> {
    let Some(tenant) = authenticated_tenant(&state, &headers).await else {
        return unauthorized().into_response();
    };
    let principal_id = derive_principal_id(&headers, tenant.slug.as_str());
    let subagent_count = headers.get("x-openai-subagent").map(|_| 1_u32).unwrap_or(0);
    let model = payload.model.clone();
    let message_summary = summarize_messages(&payload.messages);
    let selection_request = LeaseSelectionRequest {
        tenant_id: tenant.id,
        principal_id: principal_id.clone(),
        model: model.clone(),
        reasoning_effort: payload.reasoning_effort.clone(),
        subagent_count,
    };
    let mut selection = state.resolve_lease(selection_request.clone()).await;
    let context = forward_context(&headers, &principal_id);
    let stream_requested = payload.stream.unwrap_or(false);
    let mut recorded_input = false;

    for attempt in 0..2 {
        let Some((lease, replay, _warmup)) = selection.take() else {
            return waiting_chat_response(
                stream_requested,
                state.config.heartbeat_seconds,
                &payload.model,
            );
        };
        if !recorded_input {
            state
                .record_context_input(
                    &principal_id,
                    &model,
                    lease.generation,
                    message_summary.clone(),
                )
                .await;
            recorded_input = true;
        }
        let replay_context = state
            .replay_context_for(&principal_id, lease.generation)
            .await;

        let Some(credential) = state.credential_for_account(&lease.account_id).await else {
            let _ = state
                .failover_account(&lease.account_id, "credential-missing", 300, true)
                .await;
            tracing::warn!(
                account_id = %lease.account_id,
                principal_id = %principal_id,
                "selected account missing credential for chat adapter, retrying hidden failover"
            );
            if attempt == 0 {
                selection = state.resolve_lease(selection_request.clone()).await;
                continue;
            }
            return waiting_chat_response(
                stream_requested,
                state.config.heartbeat_seconds,
                &payload.model,
            );
        };

        let codex_protocol = is_codex_chatgpt_backend(&credential.base_url);
        let upstream_value = responses_payload_from_chat_request(
            &payload,
            replay.cache_key.clone(),
            replay_context.as_deref(),
            codex_protocol,
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
                    match upstream_responses_stream_to_chat_response(
                        response,
                        state.clone(),
                        lease.clone(),
                        principal_id.clone(),
                        &payload.model,
                        near_quota_guard,
                        state.config.heartbeat_seconds,
                    )
                    .await
                    {
                        ForwardOutcome::Response(response, _) => return response,
                        ForwardOutcome::HiddenFailure(kind) => {
                            handle_hidden_failure(&state, &lease, kind).await;
                            if attempt == 0 && kind.requires_failover() {
                                selection = state.resolve_lease(selection_request.clone()).await;
                                continue;
                            }
                            return waiting_chat_response(
                                stream_requested,
                                state.config.heartbeat_seconds,
                                &payload.model,
                            );
                        }
                    }
                }
                match if codex_protocol {
                    upstream_stream_to_chat_json_response(response, &payload.model).await
                } else {
                    upstream_responses_json_to_chat_response(response, &payload.model).await
                } {
                    ForwardOutcome::Response(response, output_summary) => {
                        let _ = state
                            .record_route_event(
                                &lease.account_id,
                                RouteEventRequest {
                                    mode: lease.route_mode,
                                    kind: "success".to_string(),
                                },
                            )
                            .await;
                        if let Some(output_summary) = output_summary {
                            state
                                .record_context_output(&principal_id, output_summary)
                                .await;
                        }
                        return response;
                    }
                    ForwardOutcome::HiddenFailure(kind) => {
                        handle_hidden_failure(&state, &lease, kind).await;
                        if attempt == 0 && kind.requires_failover() {
                            selection = state.resolve_lease(selection_request.clone()).await;
                            continue;
                        }
                        return waiting_chat_response(
                            stream_requested,
                            state.config.heartbeat_seconds,
                            &payload.model,
                        );
                    }
                }
            }
            Err(error) => {
                handle_hidden_failure(&state, &lease, error.kind).await;
                tracing::warn!(
                    account_id = %lease.account_id,
                    route_mode = %lease.route_mode.as_str(),
                    status = ?error.status,
                    kind = ?error.kind,
                    body_preview = %truncate_text(error.body.unwrap_or_default(), 160),
                    "chat upstream request failed"
                );
                if attempt == 0 && error.kind.requires_failover() {
                    selection = state.resolve_lease(selection_request.clone()).await;
                    continue;
                }
                return waiting_chat_response(
                    stream_requested,
                    state.config.heartbeat_seconds,
                    &payload.model,
                );
            }
        }
    }

    waiting_chat_response(
        stream_requested,
        state.config.heartbeat_seconds,
        &payload.model,
    )
}

async fn authenticated_tenant(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<crate::models::Tenant> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    state.tenant_for_bearer(token).await
}

fn waiting_response(stream_requested: bool, heartbeat_seconds: u64) -> Response<Body> {
    if stream_requested {
        let wait_stream = stream! {
            let response_id = format!("resp_wait_{}", uuid::Uuid::new_v4().simple());
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
                    .event("response.output_text.delta")
                    .data(json!({
                        "type": "response.output_text.delta",
                        "delta": "Gateway queue active. Waiting for an exact-capability account."
                    }).to_string())
            );
            yield Ok::<Event, Infallible>(
                Event::default()
                    .event("response.completed")
                    .data(json!({
                        "type": "response.completed",
                        "response": {
                            "id": response_id,
                            "status": "completed",
                            "output": [{
                                "type": "message",
                                "role": "assistant",
                                "content": [{
                                    "type": "output_text",
                                    "text": "Gateway queue active. Waiting for an exact-capability account."
                                }]
                            }]
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
            "error": {
                "message": "Gateway queue active.",
                "type": "server_busy"
            }
        })),
    )
        .into_response()
}

fn waiting_chat_response(
    stream_requested: bool,
    heartbeat_seconds: u64,
    model: &str,
) -> Response<Body> {
    if stream_requested {
        let created = Utc::now().timestamp();
        let model = model.to_string();
        let chat_id = format!("chatcmpl_wait_{}", uuid::Uuid::new_v4().simple());
        let wait_stream = stream! {
            yield Ok::<Event, Infallible>(chat_completion_sse_event(chat_completion_chunk(
                &chat_id,
                &model,
                created,
                json!({"role":"assistant"}),
                None,
            )));
            yield Ok::<Event, Infallible>(chat_completion_sse_event(chat_completion_chunk(
                &chat_id,
                &model,
                created,
                json!({"content":"Gateway queue active. Waiting for an exact-capability account."}),
                None,
            )));
            yield Ok::<Event, Infallible>(chat_completion_sse_event(chat_completion_chunk(
                &chat_id,
                &model,
                created,
                json!({}),
                Some("stop"),
            )));
            yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
        };
        return Sse::new(wait_stream)
            .keep_alive(
                KeepAlive::new()
                    .interval(Duration::from_secs(heartbeat_seconds))
                    .text("heartbeat"),
            )
            .into_response();
    }

    waiting_response(false, heartbeat_seconds)
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
    let mut builder = Response::builder().status(status);
    copy_upstream_headers(&mut builder, &headers);
    let response = builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")));
    ForwardOutcome::Response(response, output_summary)
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
    let payload = responses_json_to_chat_completion(&value, fallback_model);
    let bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    let mut builder = Response::builder().status(status);
    copy_upstream_headers(&mut builder, &headers);
    let response = builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")));
    ForwardOutcome::Response(
        response,
        (!output_summary.is_empty()).then_some(output_summary),
    )
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
    let bytes = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let mut builder = Response::builder().status(status);
    if let Some(out) = builder.headers_mut() {
        copy_upstream_headers_to_response(out, &headers);
        out.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    let response = builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::from("upstream response error")));
    ForwardOutcome::Response(
        response,
        (!output_summary.is_empty()).then_some(output_summary),
    )
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
    ForwardOutcome::Response(
        response,
        (!output_summary.is_empty()).then_some(output_summary),
    )
}

async fn upstream_stream_response(
    response: reqwest::Response,
    state: AppState,
    lease: CliLease,
    principal_id: String,
    expected_model: &str,
    _heartbeat_seconds: u64,
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
        let mut upstream = upstream;
        let mut buffer = initial_buffer;
        let mut had_hidden_failure = false;
        let mut output_summary = String::new();

        for record in buffered_records {
            if let Some(delta) = response_delta_text(&record) {
                output_summary.push_str(delta.as_str());
            }
            yield Ok::<Bytes, Infallible>(Bytes::from(format!("{record}\n\n")));
        }

        while let Some(chunk) = upstream.next().await {
            let Ok(chunk) = chunk else {
                had_hidden_failure = true;
                handle_hidden_failure(&state, &lease, UpstreamFailureKind::Generic).await;
                break;
            };
            buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
            while let Some(record) = take_sse_record(&mut buffer) {
                if let Some(kind) = hidden_failure_kind_from_sse_record(&record, &expected_model) {
                    had_hidden_failure = true;
                    handle_hidden_failure(&state, &lease, kind).await;
                    break;
                }
                if let Some(delta) = response_delta_text(&record) {
                    output_summary.push_str(delta.as_str());
                }
                yield Ok::<Bytes, Infallible>(Bytes::from(format!("{record}\n\n")));
            }
            if had_hidden_failure {
                break;
            }
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
            state.record_context_output(&principal_id, summary).await;
        }
    };
    let response = builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| Response::new(Body::from("upstream stream error")));
    ForwardOutcome::Response(response, None)
}

fn passthrough_stream_response(
    response: reqwest::Response,
    state: AppState,
    lease: CliLease,
    principal_id: String,
    expected_model: &str,
) -> ForwardOutcome {
    let status = response.status();
    let headers = response.headers().clone();
    if let Some(kind) = hidden_failure_kind_from_headers(&headers, expected_model) {
        return ForwardOutcome::HiddenFailure(kind);
    }
    let mut builder = Response::builder().status(status);
    copy_upstream_headers(&mut builder, &headers);
    let _ = tokio::spawn(async move {
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
            .record_context_output(
                &principal_id,
                "streamed assistant response delivered".to_string(),
            )
            .await;
    });
    let response = builder
        .body(Body::from_stream(
            response.bytes_stream().map_err(std::io::Error::other),
        ))
        .unwrap_or_else(|_| Response::new(Body::from("upstream stream error")));
    ForwardOutcome::Response(response, None)
}

async fn upstream_responses_stream_to_chat_response(
    response: reqwest::Response,
    state: AppState,
    lease: CliLease,
    principal_id: String,
    fallback_model: &str,
    near_quota_guard: bool,
    heartbeat_seconds: u64,
) -> ForwardOutcome {
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
    let events = stream! {
        let gateway_state = state.clone();
        let mut stream = upstream;
        let mut buffer = initial_buffer;
        let mut adapter_state = ChatStreamAdapterState::new(&fallback_model, created);
        let mut had_hidden_failure = false;
        for record in buffered_records {
            for event in translate_response_record_to_chat_events(&record, &mut adapter_state) {
                yield Ok::<Event, Infallible>(event);
            }
        }
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else {
                had_hidden_failure = true;
                handle_hidden_failure(&gateway_state, &lease, UpstreamFailureKind::Generic).await;
                break;
            };
            buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
            while let Some(record) = take_sse_record(&mut buffer) {
                if let Some(kind) = hidden_failure_kind_from_sse_record(&record, &fallback_model) {
                    had_hidden_failure = true;
                    handle_hidden_failure(&gateway_state, &lease, kind).await;
                    break;
                }
                for event in translate_response_record_to_chat_events(&record, &mut adapter_state) {
                    yield Ok::<Event, Infallible>(event);
                }
            }
            if had_hidden_failure {
                break;
            }
        }
        if !adapter_state.finished {
            yield Ok::<Event, Infallible>(chat_completion_sse_event(chat_completion_chunk(
                &adapter_state.chat_id,
                &adapter_state.model,
                adapter_state.created,
                json!({}),
                Some("stop"),
            )));
            yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
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
            gateway_state.record_context_output(&principal_id, summary).await;
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
    ForwardOutcome::Response(output, None)
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
    let mut completed_response = None;

    while let Some(chunk) = upstream.next().await {
        let chunk = chunk.map_err(|_| UpstreamFailureKind::Generic)?;
        buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()).replace("\r\n", "\n"));
        while let Some(record) = take_sse_record(&mut buffer) {
            if let Some(kind) = hidden_failure_kind_from_sse_record(&record, expected_model) {
                return Err(kind);
            }
            if let Some(value) = response_value_from_sse_record(&record)
                && value
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "response.completed")
            {
                completed_response = value.get("response").cloned().or(Some(value));
            }
        }
    }

    if completed_response.is_none() && !buffer.trim().is_empty() {
        let record = buffer.trim();
        if let Some(kind) = hidden_failure_kind_from_sse_record(record, expected_model) {
            return Err(kind);
        }
        if let Some(value) = response_value_from_sse_record(record)
            && value
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "response.completed")
        {
            completed_response = value.get("response").cloned().or(Some(value));
        }
    }

    completed_response
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
        UpstreamFailureKind::Quota | UpstreamFailureKind::Capability => {
            let _ = state
                .failover_account(
                    &lease.account_id,
                    kind.severity(),
                    kind.cooldown_seconds(),
                    false,
                )
                .await;
        }
        UpstreamFailureKind::Generic => {}
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
                !suffix.is_empty()
                    && suffix
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || ch == '-')
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
        "response.output_text.delta"
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

fn is_codex_chatgpt_backend(base_url: &str) -> bool {
    base_url
        .to_ascii_lowercase()
        .contains("/backend-api/codex")
}

fn responses_payload_for_upstream(
    payload: &ResponsesRequest,
    cache_key: String,
    replay_context: Option<&str>,
    codex_protocol: bool,
) -> Value {
    if codex_protocol {
        return codex_responses_payload(payload, cache_key, replay_context);
    }

    let mut upstream_payload = payload.clone();
    if upstream_payload.prompt_cache_key.is_none() {
        upstream_payload.prompt_cache_key = Some(cache_key);
    }
    if let Some(replay_context) = replay_context {
        upstream_payload.input =
            attach_replay_context_to_responses_input(upstream_payload.input.clone(), replay_context);
    }
    serde_json::to_value(&upstream_payload).unwrap_or_else(|_| {
        json!({
            "model": payload.model,
            "input": payload.input,
            "stream": payload.stream.unwrap_or(false)
        })
    })
}

fn codex_responses_payload(
    payload: &ResponsesRequest,
    cache_key: String,
    replay_context: Option<&str>,
) -> Value {
    let input = replay_context
        .map(|context| attach_replay_context_to_responses_input(payload.input.clone(), context))
        .unwrap_or_else(|| Value::Array(normalize_responses_input(payload.input.clone())));
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
        Value::String(
            payload
                .prompt_cache_key
                .clone()
                .unwrap_or(cache_key),
        ),
    );
    if let Some(reasoning) = payload.reasoning.clone() {
        object.insert("reasoning".to_string(), reasoning);
    }
    for (key, value) in &payload.extra {
        if !matches!(
            key.as_str(),
            "instructions" | "store" | "stream" | "prompt_cache_key"
        ) && !object.contains_key(key)
        {
            object.insert(key.clone(), value.clone());
        }
    }
    Value::Object(object)
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
    replay_context: Option<&str>,
    codex_protocol: bool,
) -> Value {
    let mut object = serde_json::Map::new();
    let mut input = payload
        .messages
        .iter()
        .map(chat_message_to_responses_input)
        .collect::<Vec<_>>();
    if let Some(replay_context) = replay_context {
        input.insert(0, replay_context_message(replay_context));
    }
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
                || !matches!(
                    key.as_str(),
                    "instructions" | "store" | "stream" | "prompt_cache_key"
                ))
        {
            object.insert(key.clone(), value.clone());
        }
    }
    Value::Object(object)
}

fn attach_replay_context_to_responses_input(input: Value, replay_context: &str) -> Value {
    let mut normalized = normalize_responses_input(input);
    normalized.insert(0, replay_context_message(replay_context));
    Value::Array(normalized)
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
    let finish_reason = if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
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
                Some(if state.saw_tool_call {
                    "tool_calls"
                } else {
                    "stop"
                }),
            )));
            out.push(Event::default().data("[DONE]"));
            out
        }
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

fn forward_context(headers: &HeaderMap, principal_id: &str) -> ForwardContext {
    let conversation_id = header_str(headers, "x-client-request-id")
        .or_else(|| header_str(headers, "session_id"))
        .or_else(|| header_str(headers, "x-codex-cli-affinity-id"))
        .unwrap_or(principal_id)
        .to_string();
    let request_id = header_str(headers, "x-client-request-id")
        .unwrap_or(conversation_id.as_str())
        .to_string();
    ForwardContext {
        conversation_id,
        request_id,
        subagent: header_str(headers, "x-openai-subagent").map(str::to_string),
        originator: header_str(headers, "originator").map(str::to_string),
    }
}

fn derive_principal_id(headers: &HeaderMap, tenant_slug: &str) -> String {
    let affinity = header_str(headers, "x-codex-cli-affinity-id")
        .or_else(|| header_str(headers, "x-client-request-id"))
        .or_else(|| header_str(headers, "session_id"))
        .or_else(|| header_str(headers, "x-openai-subagent"))
        .unwrap_or("anonymous");
    format!("tenant:{tenant_slug}/principal:{affinity}")
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

        let mapped =
            responses_payload_from_chat_request(&payload, "cache-123".to_string(), None, false);
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

        let mapped = responses_payload_from_chat_request(
            &payload,
            "cache-123".to_string(),
            Some("[cmgr replay context]\nrecent_turns=1"),
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

        let mapped =
            responses_payload_from_chat_request(&payload, "cache-123".to_string(), None, true);
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

        let mapped = responses_payload_for_upstream(&payload, "cache-123".to_string(), None, true);
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

        let kind = hidden_failure_kind_from_json(
            &payload,
            "gpt-5.4",
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(kind, Some(UpstreamFailureKind::Quota));
    }

    #[test]
    fn hidden_failure_detects_model_drift() {
        let payload = json!({
            "id": "resp_drift",
            "status": "completed",
            "model": "gpt-4.1-mini"
        });

        let kind = hidden_failure_kind_from_json(
            &payload,
            "gpt-5.4",
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(kind, Some(UpstreamFailureKind::Capability));
    }

    #[test]
    fn versioned_model_alias_does_not_count_as_drift() {
        let payload = json!({
            "id": "resp_alias",
            "status": "completed",
            "model": "gpt-5.2-2025-12-11"
        });

        let kind = hidden_failure_kind_from_json(
            &payload,
            "gpt-5.2",
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(kind, None);
        assert!(model_matches_expected("gpt-5.3-codex-2025-12-11", "gpt-5.3-codex"));
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
