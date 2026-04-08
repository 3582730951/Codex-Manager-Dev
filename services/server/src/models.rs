use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    Direct,
    Warp,
}

impl RouteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Warp => "warp",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "warp" => Self::Warp,
            _ => Self::Direct,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GatewayUserRole {
    Admin,
    Viewer,
}

impl GatewayUserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Viewer => "viewer",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "admin" => Self::Admin,
            _ => Self::Viewer,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tenant {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayApiKey {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: GatewayUserRole,
    pub token: String,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub force_model_override: bool,
    pub force_reasoning_effort: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulingSignals {
    pub quota_headroom: f64,
    pub quota_headroom_5h: f64,
    pub quota_headroom_7d: f64,
    pub health_score: f64,
    pub egress_stability: f64,
    pub fairness_bias: f64,
    pub inflight: u32,
    pub capacity: u32,
}

impl SchedulingSignals {
    pub fn effective_quota_headroom(&self) -> f64 {
        self.quota_headroom
            .min(self.quota_headroom_5h)
            .min(self.quota_headroom_7d)
    }

    pub fn near_quota_guard_enabled(&self) -> bool {
        self.quota_headroom_5h < 0.30 || self.quota_headroom_7d < 0.30
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamAccount {
    pub id: String,
    pub tenant_id: Uuid,
    pub label: String,
    pub models: Vec<String>,
    pub current_mode: RouteMode,
    pub signals: SchedulingSignals,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamCredential {
    pub account_id: String,
    pub base_url: String,
    pub bearer_token: String,
    pub chatgpt_account_id: Option<String>,
    pub extra_headers: Vec<(String, String)>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRouteState {
    pub account_id: String,
    pub route_mode: RouteMode,
    pub direct_cf_streak: u32,
    pub warp_cf_streak: u32,
    pub cooldown_level: usize,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub warp_entered_at: Option<DateTime<Utc>>,
    pub last_cf_at: Option<DateTime<Utc>>,
    pub success_streak: u32,
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliLease {
    pub principal_id: String,
    pub tenant_id: Uuid,
    pub account_id: String,
    pub account_label: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub route_mode: RouteMode,
    pub generation: u32,
    pub active_subagents: u32,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextTurn {
    pub generation: u32,
    pub request_summary: String,
    pub response_summary: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationContext {
    pub principal_id: String,
    pub model: String,
    pub workflow_spine: String,
    pub turns: Vec<ContextTurn>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheMetrics {
    pub cached_tokens: u64,
    pub replay_tokens: u64,
    pub prefix_hit_ratio: f64,
    pub warmup_roi: f64,
    pub static_prefix_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfIncident {
    pub id: String,
    pub account_id: String,
    pub account_label: String,
    pub route_mode: RouteMode,
    pub severity: String,
    pub happened_at: DateTime<Utc>,
    pub cooldown_level: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyNode {
    pub name: String,
    pub purpose: String,
    pub hot_path: bool,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardCounts {
    pub tenants: usize,
    pub accounts: usize,
    pub users: usize,
    pub active_leases: usize,
    pub warp_accounts: usize,
    pub browser_tasks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub id: String,
    pub tenant_id: Uuid,
    pub label: String,
    pub models: Vec<String>,
    pub current_mode: RouteMode,
    pub route_mode: RouteMode,
    pub cooldown_level: usize,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub direct_cf_streak: u32,
    pub warp_cf_streak: u32,
    pub success_streak: u32,
    pub quota_headroom: f64,
    pub quota_headroom_5h: f64,
    pub quota_headroom_7d: f64,
    pub near_quota_guard_enabled: bool,
    pub health_score: f64,
    pub egress_stability: f64,
    pub inflight: u32,
    pub capacity: u32,
    pub has_credential: bool,
    pub base_url: Option<String>,
    pub chatgpt_account_id: Option<String>,
    pub egress_group: String,
    pub proxy_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTask {
    pub id: String,
    pub kind: String,
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub provider: Option<String>,
    pub route_mode: Option<RouteMode>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub notes: Option<String>,
    pub profile_dir: Option<String>,
    pub screenshot_path: Option<String>,
    pub storage_state_path: Option<String>,
    pub final_url: Option<String>,
    pub last_error: Option<String>,
    pub step_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiLoginSessionView {
    pub login_id: String,
    pub tenant_id: Uuid,
    pub label: Option<String>,
    pub note: Option<String>,
    pub redirect_uri: String,
    pub auth_url: String,
    pub status: String,
    pub error: Option<String>,
    pub imported_account_id: Option<String>,
    pub imported_account_label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressSlot {
    pub id: String,
    pub route_mode: RouteMode,
    pub configured: bool,
    pub upstream_proxy_url_preview: Option<String>,
    pub browser_proxy_url_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayApiKeyView {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: GatewayUserRole,
    pub token_preview: String,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub force_model_override: bool,
    pub force_reasoning_effort: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedGatewayApiKey {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: GatewayUserRole,
    pub token: String,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub force_model_override: bool,
    pub force_reasoning_effort: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingSummary {
    pub total_spend_usd: f64,
    pub total_requests: usize,
    pub total_input_tokens: u64,
    pub total_cached_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub priced_requests: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogEntry {
    pub id: String,
    pub api_key_id: Uuid,
    pub tenant_id: Uuid,
    pub user_name: String,
    pub user_email: String,
    pub principal_id: String,
    pub account_id: String,
    pub account_label: String,
    pub method: String,
    pub endpoint: String,
    pub requested_model: String,
    pub effective_model: String,
    pub reasoning_effort: Option<String>,
    pub route_mode: RouteMode,
    pub status_code: u16,
    pub usage: RequestLogUsage,
    pub estimated_cost_usd: Option<f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUserView {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: GatewayUserRole,
    pub token_preview: String,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub force_model_override: bool,
    pub force_reasoning_effort: bool,
    pub request_count: usize,
    pub estimated_spend_usd: f64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedGatewayUser {
    pub user: GatewayUserView,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub title: String,
    pub subtitle: String,
    pub topology: Vec<TopologyNode>,
    pub cache_metrics: CacheMetrics,
    pub accounts: Vec<AccountSummary>,
    pub leases: Vec<CliLease>,
    pub cf_incidents: Vec<CfIncident>,
    pub browser_tasks: Vec<BrowserTask>,
    pub users: Vec<GatewayUserView>,
    pub request_logs: Vec<RequestLogEntry>,
    pub billing: BillingSummary,
    pub model_catalog: Vec<String>,
    pub counts: DashboardCounts,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTenantRequest {
    pub slug: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAccountRequest {
    pub tenant_id: Uuid,
    pub label: String,
    pub models: Vec<String>,
    pub quota_headroom: Option<f64>,
    pub quota_headroom_5h: Option<f64>,
    pub quota_headroom_7d: Option<f64>,
    pub health_score: Option<f64>,
    pub egress_stability: Option<f64>,
    pub base_url: Option<String>,
    pub bearer_token: Option<String>,
    pub chatgpt_account_id: Option<String>,
    pub extra_headers: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGatewayApiKeyRequest {
    pub tenant_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGatewayUserRequest {
    pub tenant_id: Option<Uuid>,
    pub name: String,
    pub email: String,
    pub role: GatewayUserRole,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub force_model_override: Option<bool>,
    pub force_reasoning_effort: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGatewayUserRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub role: Option<GatewayUserRole>,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub force_model_override: Option<bool>,
    pub force_reasoning_effort: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTaskRequest {
    pub account_id: Option<String>,
    pub notes: Option<String>,
    pub login_url: Option<String>,
    pub headless: Option<bool>,
    pub provider: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
    pub otp_code: Option<String>,
    pub route_mode: Option<RouteMode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiLoginStartRequest {
    pub tenant_id: Option<Uuid>,
    pub label: Option<String>,
    pub note: Option<String>,
    pub redirect_uri: String,
    pub models: Option<Vec<String>>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiLoginStartResponse {
    pub login_id: String,
    pub auth_url: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiLoginCompleteRequest {
    pub state: String,
    pub code: String,
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEventRequest {
    pub mode: RouteMode,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponsesRequest {
    pub model: String,
    pub input: Value,
    pub stream: Option<bool>,
    pub reasoning: Option<Value>,
    pub prompt_cache_key: Option<String>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: Option<bool>,
    pub reasoning_effort: Option<String>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Clone)]
pub struct LeaseSelectionRequest {
    pub tenant_id: Uuid,
    pub principal_id: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub subagent_count: u32,
}

#[cfg(test)]
mod tests {
    use super::SchedulingSignals;

    #[test]
    fn near_quota_guard_only_enables_for_low_5h_or_7d_windows() {
        let healthy = SchedulingSignals {
            quota_headroom: 0.9,
            quota_headroom_5h: 0.31,
            quota_headroom_7d: 0.31,
            health_score: 0.8,
            egress_stability: 0.8,
            fairness_bias: 0.7,
            inflight: 0,
            capacity: 4,
        };
        assert!(!healthy.near_quota_guard_enabled());

        let low_5h = SchedulingSignals {
            quota_headroom_5h: 0.29,
            ..healthy.clone()
        };
        assert!(low_5h.near_quota_guard_enabled());

        let low_7d = SchedulingSignals {
            quota_headroom_5h: 0.9,
            quota_headroom_7d: 0.29,
            ..healthy
        };
        assert!(low_7d.near_quota_guard_enabled());
    }
}
