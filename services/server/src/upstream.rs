use std::time::Duration;

use axum::http::header::ACCEPT;
use reqwest::{Client, Proxy, Response, StatusCode};
use serde_json::Value;
use tracing::warn;

use crate::{
    config::Config,
    models::{RouteMode, UpstreamCredential},
};

const UNARY_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const MODEL_CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct ForwardContext {
    pub conversation_id: String,
    pub request_id: String,
    pub subagent: Option<String>,
    pub originator: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamFailureKind {
    Cf,
    Auth,
    Quota,
    Capability,
    Generic,
}

impl UpstreamFailureKind {
    pub fn requires_failover(self) -> bool {
        matches!(self, Self::Cf | Self::Auth | Self::Quota | Self::Capability)
    }

    pub fn cooldown_seconds(self) -> i64 {
        match self {
            Self::Cf => 300,
            Self::Auth => 300,
            Self::Quota => 1800,
            Self::Capability => 300,
            Self::Generic => 0,
        }
    }

    pub fn severity(self) -> &'static str {
        match self {
            Self::Cf => "cf",
            Self::Auth => "auth",
            Self::Quota => "quota",
            Self::Capability => "capability",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpstreamFailure {
    pub status: Option<StatusCode>,
    pub body: Option<String>,
    pub kind: UpstreamFailureKind,
}

#[derive(Clone)]
pub struct UpstreamClient {
    default_client: Client,
    direct_client: Option<Client>,
    warp_client: Option<Client>,
}

impl UpstreamClient {
    pub fn new(config: &Config) -> Self {
        let default_client = build_client(None).expect("build default upstream client");
        let direct_client = config
            .direct_proxy_url
            .as_deref()
            .map(|proxy_url| build_client(Some(proxy_url)))
            .transpose()
            .unwrap_or_else(|error| {
                warn!(%error, "failed to build direct proxy client, falling back to native egress");
                None
            });
        let warp_client = config
            .warp_proxy_url
            .as_deref()
            .map(|proxy_url| build_client(Some(proxy_url)))
            .transpose()
            .unwrap_or_else(|error| {
                warn!(%error, "failed to build warp proxy client, falling back to native egress");
                None
            });
        Self {
            default_client,
            direct_client,
            warp_client,
        }
    }

    pub async fn post_json(
        &self,
        credential: &UpstreamCredential,
        path: &str,
        payload: &Value,
        context: &ForwardContext,
        stream: bool,
        route_mode: RouteMode,
    ) -> Result<Response, UpstreamFailure> {
        let url = endpoint_url(&credential.base_url, path);
        let mut request = self
            .client_for_route_mode(route_mode)
            .post(url)
            .bearer_auth(&credential.bearer_token)
            .header("session_id", &context.conversation_id)
            .header("x-client-request-id", &context.request_id)
            .json(payload);

        if let Some(timeout) = request_timeout_for_stream(stream) {
            request = request.timeout(timeout);
        }

        if stream {
            request = request.header(ACCEPT, "text/event-stream");
        }

        if let Some(account_id) = credential.chatgpt_account_id.as_deref() {
            request = request.header("ChatGPT-Account-ID", account_id);
        }
        if let Some(subagent) = context.subagent.as_deref() {
            request = request.header("x-openai-subagent", subagent);
        }
        if let Some(originator) = context.originator.as_deref() {
            request = request.header("originator", originator);
        }

        for (name, value) in &credential.extra_headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(|error| {
            warn!(%error, account_id = %credential.account_id, "upstream request failed before response");
            UpstreamFailure {
                status: None,
                body: None,
                kind: UpstreamFailureKind::Generic,
            }
        })?;

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();
        let kind = classify_failure(status, &headers, &body);
        Err(UpstreamFailure {
            status: Some(status),
            body: Some(body),
            kind,
        })
    }

    pub async fn list_models(
        &self,
        credential: &UpstreamCredential,
        route_mode: RouteMode,
    ) -> Result<Vec<String>, String> {
        let url = endpoint_url(&credential.base_url, "v1/models");
        let mut request = self
            .client_for_route_mode(route_mode)
            .get(url)
            .bearer_auth(&credential.bearer_token)
            .timeout(MODEL_CATALOG_TIMEOUT);

        if let Some(account_id) = credential.chatgpt_account_id.as_deref() {
            request = request.header("ChatGPT-Account-ID", account_id);
        }
        for (name, value) in &credential.extra_headers {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .map_err(|error| format!("upstream model catalog request failed: {error}"))?;
        let status = response.status();
        let value = response
            .json::<Value>()
            .await
            .map_err(|error| format!("invalid upstream model catalog payload: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "upstream model catalog returned {}: {}",
                status,
                value
            ));
        }

        let items = value
            .get("data")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(items)
    }

    fn client_for_route_mode(&self, route_mode: RouteMode) -> &Client {
        match route_mode {
            RouteMode::Direct => self.direct_client.as_ref().unwrap_or(&self.default_client),
            RouteMode::Warp => self.warp_client.as_ref().unwrap_or(&self.default_client),
        }
    }
}

impl Default for UpstreamClient {
    fn default() -> Self {
        Self {
            default_client: build_client(None).expect("build default upstream client"),
            direct_client: None,
            warp_client: None,
        }
    }
}

fn build_client(proxy_url: Option<&str>) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        .tcp_nodelay(true)
        .user_agent("codex-manager/0.1");
    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(Proxy::all(proxy_url)?);
    }
    builder.build()
}

fn request_timeout_for_stream(stream: bool) -> Option<Duration> {
    (!stream).then_some(UNARY_REQUEST_TIMEOUT)
}

pub fn endpoint_url(base_url: &str, path: &str) -> String {
    let normalized_base = base_url.trim_end_matches('/');
    let normalized_path = path.trim_start_matches('/');
    if normalized_base.ends_with(normalized_path) {
        normalized_base.to_string()
    } else {
        format!("{normalized_base}/{normalized_path}")
    }
}

pub fn looks_like_cf(status: StatusCode, headers: &reqwest::header::HeaderMap, body: &str) -> bool {
    if !(status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS) {
        return false;
    }
    let body = body.to_ascii_lowercase();
    if headers.contains_key("cf-ray") {
        return true;
    }
    [
        "cloudflare",
        "cf-ray",
        "attention required",
        "challenge-platform",
        "/cdn-cgi/challenge-platform",
        "just a moment",
    ]
    .iter()
    .any(|needle| body.contains(needle))
}

pub fn classify_failure(
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> UpstreamFailureKind {
    let body_kind = classify_failure_body(body);

    if status == StatusCode::UNAUTHORIZED
        || matches!(body_kind, Some(UpstreamFailureKind::Auth))
    {
        return UpstreamFailureKind::Auth;
    }
    if matches!(body_kind, Some(UpstreamFailureKind::Quota)) {
        return UpstreamFailureKind::Quota;
    }
    if matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND)
        && matches!(body_kind, Some(UpstreamFailureKind::Capability))
    {
        return UpstreamFailureKind::Capability;
    }
    if looks_like_cf(status, headers, body) {
        return UpstreamFailureKind::Cf;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return UpstreamFailureKind::Quota;
    }
    if let Some(kind) = body_kind {
        return kind;
    }
    UpstreamFailureKind::Generic
}

pub fn classify_failure_body(body: &str) -> Option<UpstreamFailureKind> {
    let lowered = body.to_ascii_lowercase();
    if [
        "cloudflare",
        "cf-ray",
        "attention required",
        "challenge-platform",
        "/cdn-cgi/challenge-platform",
        "just a moment",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
    {
        return Some(UpstreamFailureKind::Cf);
    }
    if [
        "invalid_api_key",
        "invalid api key",
        "authentication",
        "unauthorized",
        "token expired",
        "auth",
        "forbidden",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
    {
        return Some(UpstreamFailureKind::Auth);
    }
    if [
        "insufficient_quota",
        "usage_limit_reached",
        "quota",
        "billing",
        "rate limit reached for this model",
        "usage limit",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
    {
        return Some(UpstreamFailureKind::Quota);
    }
    if [
        "reasoning_effort",
        "does not support",
        "unsupported",
        "model not found",
        "unknown model",
        "model_not_found",
        "unknown_model",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
    {
        return Some(UpstreamFailureKind::Capability);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct Recorder {
        headers: Arc<Mutex<Vec<HeaderMap>>>,
        bodies: Arc<Mutex<Vec<Value>>>,
    }

    async fn record_request(
        State(recorder): State<Recorder>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        recorder.headers.lock().await.push(headers);
        recorder.bodies.lock().await.push(payload);
        Json(json!({
            "id": format!("resp_{}", Utc::now().timestamp_nanos_opt().unwrap_or_default()),
            "status": "completed"
        }))
    }

    #[tokio::test]
    async fn post_json_adds_codex_headers() {
        let recorder = Recorder::default();
        let app = Router::new()
            .route("/v1/responses", post(record_request))
            .with_state(recorder.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        let client = UpstreamClient::new(&Config {
            bind_addr: "127.0.0.1".parse().expect("ip"),
            data_port: 8080,
            admin_port: 8081,
            postgres_url: "postgres://localhost/test".to_string(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
            redis_channel: "cmgr:test".to_string(),
            instance_id: "cmgr-test".to_string(),
            browser_assist_url: "http://127.0.0.1:8090".to_string(),
            heartbeat_seconds: 5,
            enable_demo_seed: false,
            direct_proxy_url: None,
            warp_proxy_url: None,
            browser_assist_direct_proxy_url: None,
            browser_assist_warp_proxy_url: None,
        });
        let credential = UpstreamCredential {
            account_id: "acc_1".to_string(),
            base_url: format!("http://{addr}/v1"),
            bearer_token: "secret".to_string(),
            chatgpt_account_id: Some("acct-live".to_string()),
            extra_headers: vec![("x-test-header".to_string(), "present".to_string())],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response = client
            .post_json(
                &credential,
                "responses",
                &json!({"model":"gpt-5.4","stream":false}),
                &ForwardContext {
                    conversation_id: "sess-123".to_string(),
                    request_id: "req-123".to_string(),
                    subagent: Some("review".to_string()),
                    originator: Some("cmgr".to_string()),
                },
                false,
                RouteMode::Direct,
            )
            .await
            .expect("success");
        assert_eq!(response.status(), StatusCode::OK);

        let headers = recorder.headers.lock().await;
        let request_headers = headers.first().expect("recorded headers");
        assert_eq!(
            request_headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer secret")
        );
        assert_eq!(
            request_headers
                .get("ChatGPT-Account-ID")
                .and_then(|value| value.to_str().ok()),
            Some("acct-live")
        );
        assert_eq!(
            request_headers
                .get("session_id")
                .and_then(|value| value.to_str().ok()),
            Some("sess-123")
        );
        assert_eq!(
            request_headers
                .get("x-client-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("req-123")
        );
        assert_eq!(
            request_headers
                .get("x-openai-subagent")
                .and_then(|value| value.to_str().ok()),
            Some("review")
        );
        assert_eq!(
            request_headers
                .get("x-test-header")
                .and_then(|value| value.to_str().ok()),
            Some("present")
        );
    }

    #[test]
    fn endpoint_url_does_not_duplicate_path() {
        assert_eq!(
            endpoint_url("https://api.openai.com/v1", "responses"),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            endpoint_url("https://api.openai.com/v1/responses", "responses"),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn streaming_requests_do_not_use_total_timeout() {
        assert_eq!(request_timeout_for_stream(true), None);
        assert_eq!(
            request_timeout_for_stream(false),
            Some(UNARY_REQUEST_TIMEOUT)
        );
    }

    #[test]
    fn cf_detection_uses_status_and_body_features() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert!(!looks_like_cf(
            StatusCode::BAD_REQUEST,
            &headers,
            "ordinary error"
        ));
        headers.insert("cf-ray", reqwest::header::HeaderValue::from_static("123"));
        assert!(!looks_like_cf(
            StatusCode::BAD_REQUEST,
            &headers,
            "ordinary error"
        ));
        assert!(looks_like_cf(StatusCode::FORBIDDEN, &headers, "blocked"));
        headers.clear();
        assert!(looks_like_cf(
            StatusCode::FORBIDDEN,
            &headers,
            "<html>Attention Required! | Cloudflare</html>"
        ));
    }

    #[test]
    fn classify_failure_detects_hard_failures() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(
            classify_failure(StatusCode::UNAUTHORIZED, &headers, "token expired"),
            UpstreamFailureKind::Auth
        );
        assert_eq!(
            classify_failure(
                StatusCode::TOO_MANY_REQUESTS,
                &headers,
                "{\"error\":\"insufficient_quota\"}"
            ),
            UpstreamFailureKind::Quota
        );
        assert_eq!(
            classify_failure(
                StatusCode::BAD_REQUEST,
                &headers,
                "model gpt-5.4 does not support reasoning_effort"
            ),
            UpstreamFailureKind::Capability
        );
    }

    #[test]
    fn classify_failure_prefers_quota_over_cf_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("cf-ray", reqwest::header::HeaderValue::from_static("123"));

        assert_eq!(
            classify_failure(
                StatusCode::TOO_MANY_REQUESTS,
                &headers,
                "{\"error\":{\"type\":\"usage_limit_reached\",\"message\":\"The usage limit has been reached\"}}"
            ),
            UpstreamFailureKind::Quota
        );
    }
}
