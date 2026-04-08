use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use tokio::{
    sync::{RwLock, mpsc, mpsc::error::TryRecvError},
    time::Duration,
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::models::{
    AccountRouteState, AccountSummary, CacheMetrics, CfIncident, CliLease, ContextTurn,
    ConversationContext, CreateGatewayApiKeyRequest, CreateGatewayUserRequest,
    CreateTenantRequest, CreatedGatewayApiKey, CreatedGatewayUser, DashboardCounts,
    DashboardSnapshot, EgressSlot, GatewayApiKey, GatewayApiKeyView, GatewayUserRole,
    GatewayUserView, ImportAccountRequest, LeaseSelectionRequest, OpenAiLoginCompleteRequest,
    OpenAiLoginSessionView, OpenAiLoginStartRequest, OpenAiLoginStartResponse, RequestLogEntry,
    RouteEventRequest, RouteMode, SchedulingSignals, Tenant, TopologyNode,
    UpdateGatewayUserRequest, UpstreamAccount, UpstreamCredential, BillingSummary,
};
use crate::openai_auth;
use crate::reasoning::normalize_reasoning_effort;
use crate::scheduler::cf_state::{
    is_in_cooldown, reconcile_route_mode, register_cf_hit, register_success,
};
use crate::scheduler::replay::{ReplayPack, compile_replay_pack};
use crate::scheduler::router::{score_candidate, select_dual_candidates, should_reuse_lease};
use crate::scheduler::token_optimizer::{WarmupDecision, evaluate_prefix_warmup};
use crate::storage::{Persistence, PersistenceMessage, PersistenceSnapshot};
use crate::upstream::UpstreamClient;
use crate::{browser_assist, bus, config::Config};

#[derive(Debug, Clone)]
pub struct GatewayAuthContext {
    pub tenant: Tenant,
    pub api_key: GatewayApiKey,
}

#[derive(Debug, Clone)]
struct OpenAiLoginSessionState {
    login_id: String,
    tenant_id: Uuid,
    label: Option<String>,
    note: Option<String>,
    redirect_uri: String,
    auth_url: String,
    code_verifier: String,
    status: String,
    error: Option<String>,
    imported_account_id: Option<String>,
    imported_account_label: Option<String>,
    models: Vec<String>,
    base_url: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

impl OpenAiLoginSessionState {
    fn view(&self) -> OpenAiLoginSessionView {
        OpenAiLoginSessionView {
            login_id: self.login_id.clone(),
            tenant_id: self.tenant_id,
            label: self.label.clone(),
            note: self.note.clone(),
            redirect_uri: self.redirect_uri.clone(),
            auth_url: self.auth_url.clone(),
            status: self.status.clone(),
            error: self.error.clone(),
            imported_account_id: self.imported_account_id.clone(),
            imported_account_label: self.imported_account_label.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Default)]
pub struct RuntimeState {
    pub tenants: RwLock<HashMap<Uuid, Tenant>>,
    pub api_keys: RwLock<HashMap<String, GatewayApiKey>>,
    pub accounts: RwLock<HashMap<String, UpstreamAccount>>,
    pub credentials: RwLock<HashMap<String, UpstreamCredential>>,
    pub route_states: RwLock<HashMap<String, AccountRouteState>>,
    pub leases: RwLock<HashMap<String, CliLease>>,
    pub cf_incidents: RwLock<Vec<CfIncident>>,
    pub cache_metrics: RwLock<CacheMetrics>,
    pub conversation_contexts: RwLock<HashMap<String, ConversationContext>>,
    pub request_logs: RwLock<Vec<RequestLogEntry>>,
    openai_login_sessions: RwLock<HashMap<String, OpenAiLoginSessionState>>,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub runtime: Arc<RuntimeState>,
    pub upstream: UpstreamClient,
    pub writer_tx: mpsc::Sender<PersistenceMessage>,
    pub bus_tx: Option<mpsc::Sender<PersistenceMessage>>,
    pub persistence: Option<Arc<Persistence>>,
    pub redis_connected: bool,
}

impl AppState {
    pub async fn new(config: Config) -> Self {
        let persistence = Persistence::connect(&config.postgres_url)
            .await
            .map(Arc::new);
        let runtime = Arc::new(RuntimeState {
            cache_metrics: RwLock::new(default_cache_metrics()),
            ..RuntimeState::default()
        });
        let (writer_tx, writer_rx) = mpsc::channel::<PersistenceMessage>(1024);
        let bus_tx = bus::connect(&config, runtime.clone()).await;
        let redis_connected = bus_tx.is_some();
        let upstream = UpstreamClient::new(&config);

        spawn_persistence_writer(writer_rx, persistence.clone());
        let state = Self {
            config,
            runtime,
            upstream,
            writer_tx,
            bus_tx,
            persistence,
            redis_connected,
        };

        let restored = if let Some(persistence) = state.persistence.as_ref() {
            match persistence.load_snapshot().await {
                Ok(snapshot) if snapshot.has_data() => {
                    state.load_snapshot(snapshot).await;
                    true
                }
                Ok(_) => false,
                Err(error) => {
                    warn!(%error, "failed to load postgres snapshot, falling back to demo seed");
                    false
                }
            }
        } else {
            false
        };

        if !restored && state.config.enable_demo_seed {
            state.seed_demo().await;
            if let Some(persistence) = state.persistence.as_ref() {
                let bootstrap_batch = state.runtime_snapshot_batch().await;
                if let Err(error) = persistence.persist_batch(&bootstrap_batch).await {
                    warn!(%error, batch_size = bootstrap_batch.len(), "failed to persist bootstrap snapshot");
                }
            }
        }

        state
    }

    pub fn postgres_connected(&self) -> bool {
        self.persistence.is_some()
    }

    pub fn redis_connected(&self) -> bool {
        self.redis_connected
    }

    async fn load_snapshot(&self, snapshot: PersistenceSnapshot) {
        let PersistenceSnapshot {
            tenants,
            api_keys,
            accounts,
            credentials,
            route_states,
            leases,
            cf_incidents,
            cache_metrics,
            conversation_contexts,
            request_logs,
        } = snapshot;

        {
            let mut tenant_map = self.runtime.tenants.write().await;
            tenant_map.clear();
            for tenant in tenants {
                tenant_map.insert(tenant.id, tenant);
            }
        }
        {
            let mut api_key_map = self.runtime.api_keys.write().await;
            api_key_map.clear();
            for api_key in api_keys {
                api_key_map.insert(api_key.token.clone(), api_key);
            }
        }
        {
            let mut account_map = self.runtime.accounts.write().await;
            account_map.clear();
            for account in accounts {
                account_map.insert(account.id.clone(), account);
            }
        }
        {
            let mut credential_map = self.runtime.credentials.write().await;
            credential_map.clear();
            for credential in credentials {
                credential_map.insert(credential.account_id.clone(), credential);
            }
        }
        {
            let mut route_state_map = self.runtime.route_states.write().await;
            route_state_map.clear();
            for route_state in route_states {
                route_state_map.insert(route_state.account_id.clone(), route_state);
            }
        }
        {
            let mut lease_map = self.runtime.leases.write().await;
            lease_map.clear();
            for lease in leases {
                lease_map.insert(lease.principal_id.clone(), lease);
            }
        }
        {
            let mut incidents = self.runtime.cf_incidents.write().await;
            *incidents = cf_incidents;
        }
        {
            let mut contexts = self.runtime.conversation_contexts.write().await;
            contexts.clear();
            for context in conversation_contexts {
                contexts.insert(context.principal_id.clone(), context);
            }
        }
        {
            let mut logs = self.runtime.request_logs.write().await;
            *logs = request_logs;
        }
        if let Some(metrics) = cache_metrics {
            *self.runtime.cache_metrics.write().await = metrics;
        }
    }

    async fn runtime_snapshot_batch(&self) -> Vec<PersistenceMessage> {
        let mut batch = Vec::new();

        let tenants = self
            .runtime
            .tenants
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        batch.extend(tenants.into_iter().map(PersistenceMessage::TenantUpsert));

        let api_keys = self
            .runtime
            .api_keys
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        batch.extend(api_keys.into_iter().map(PersistenceMessage::ApiKeyUpsert));

        let accounts = self
            .runtime
            .accounts
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        batch.extend(accounts.into_iter().map(PersistenceMessage::AccountUpsert));

        let credentials = self
            .runtime
            .credentials
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        batch.extend(
            credentials
                .into_iter()
                .map(PersistenceMessage::CredentialUpsert),
        );

        let route_states = self
            .runtime
            .route_states
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        batch.extend(
            route_states
                .into_iter()
                .map(PersistenceMessage::RouteStateUpsert),
        );

        let leases = self
            .runtime
            .leases
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        batch.extend(leases.into_iter().map(PersistenceMessage::LeaseUpsert));

        let incidents = self.runtime.cf_incidents.read().await.clone();
        batch.extend(
            incidents
                .into_iter()
                .map(PersistenceMessage::IncidentInsert),
        );

        let contexts = self
            .runtime
            .conversation_contexts
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        batch.extend(
            contexts
                .into_iter()
                .map(PersistenceMessage::ConversationContextUpsert),
        );

        let request_logs = self.runtime.request_logs.read().await.clone();
        batch.extend(
            request_logs
                .into_iter()
                .map(PersistenceMessage::RequestLogInsert),
        );

        batch.push(PersistenceMessage::CacheMetricsUpsert(
            self.runtime.cache_metrics.read().await.clone(),
        ));
        batch
    }

    async fn seed_demo(&self) {
        let tenant = Tenant {
            id: Uuid::new_v4(),
            slug: "demo".to_string(),
            name: "Demo Tenant".to_string(),
            created_at: Utc::now(),
        };
        self.runtime
            .tenants
            .write()
            .await
            .insert(tenant.id, tenant.clone());
        self.runtime.api_keys.write().await.insert(
            "cmgr_demo_key".to_string(),
            GatewayApiKey {
                id: Uuid::new_v4(),
                tenant_id: tenant.id,
                name: "Demo Gateway Key".to_string(),
                email: "demo@codex.local".to_string(),
                role: GatewayUserRole::Admin,
                token: "cmgr_demo_key".to_string(),
                default_model: Some("gpt-5.4".to_string()),
                reasoning_effort: Some("high".to_string()),
                force_model_override: false,
                force_reasoning_effort: false,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        );

        for account in [
            demo_account(
                &tenant.id,
                "acc_demo_1",
                "Meridian",
                RouteMode::Direct,
                0.92,
                0.96,
                0.88,
            ),
            demo_account(
                &tenant.id,
                "acc_demo_2",
                "Mistral Wing",
                RouteMode::Warp,
                0.83,
                0.82,
                0.94,
            ),
            demo_account(
                &tenant.id,
                "acc_demo_3",
                "Copperline",
                RouteMode::Direct,
                0.74,
                0.89,
                0.71,
            ),
            demo_account(
                &tenant.id,
                "acc_demo_4",
                "Delta North",
                RouteMode::Direct,
                0.61,
                0.78,
                0.67,
            ),
        ] {
            self.runtime.route_states.write().await.insert(
                account.id.clone(),
                AccountRouteState {
                    account_id: account.id.clone(),
                    route_mode: account.current_mode,
                    direct_cf_streak: 0,
                    warp_cf_streak: if account.current_mode == RouteMode::Warp {
                        1
                    } else {
                        0
                    },
                    cooldown_level: if account.current_mode == RouteMode::Warp {
                        2
                    } else {
                        0
                    },
                    cooldown_until: None,
                    warp_entered_at: if account.current_mode == RouteMode::Warp {
                        Some(Utc::now())
                    } else {
                        None
                    },
                    last_cf_at: None,
                    success_streak: 0,
                    last_success_at: None,
                },
            );
            self.runtime
                .accounts
                .write()
                .await
                .insert(account.id.clone(), account);
        }

        self.runtime.leases.write().await.insert(
            "tenant:demo/principal:atlas-shell".to_string(),
            CliLease {
                principal_id: "tenant:demo/principal:atlas-shell".to_string(),
                tenant_id: tenant.id,
                account_id: "acc_demo_1".to_string(),
                account_label: "Meridian".to_string(),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                route_mode: RouteMode::Direct,
                generation: 8,
                active_subagents: 3,
                created_at: Utc::now(),
                last_used_at: Utc::now(),
            },
        );
        self.runtime.leases.write().await.insert(
            "tenant:demo/principal:review-bot".to_string(),
            CliLease {
                principal_id: "tenant:demo/principal:review-bot".to_string(),
                tenant_id: tenant.id,
                account_id: "acc_demo_2".to_string(),
                account_label: "Mistral Wing".to_string(),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                route_mode: RouteMode::Warp,
                generation: 3,
                active_subagents: 1,
                created_at: Utc::now(),
                last_used_at: Utc::now(),
            },
        );
    }

    pub async fn dashboard_snapshot(&self) -> DashboardSnapshot {
        let tenants = self
            .runtime
            .tenants
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let accounts = self
            .runtime
            .accounts
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let route_states = self
            .runtime
            .route_states
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut leases = self
            .runtime
            .leases
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        leases.sort_by(|left, right| right.last_used_at.cmp(&left.last_used_at));

        let cf_incidents = self.runtime.cf_incidents.read().await.clone();
        let cache_metrics = self.runtime.cache_metrics.read().await.clone();
        let account_summaries = self.account_summaries().await;
        let request_logs = self.list_request_logs().await;
        let billing = self.billing_summary(&request_logs);
        let users = self.gateway_user_views(&request_logs).await;
        let model_catalog = build_model_catalog(&account_summaries);
        DashboardSnapshot {
            title: "Codex Manager 2.0".to_string(),
            subtitle:
                "Responses-first, lease-bound routing, dual-candidate selection, and warp-aware recovery."
                    .to_string(),
            topology: vec![
                TopologyNode {
                    name: "web".to_string(),
                    purpose: "Node SSR operations surface".to_string(),
                    hot_path: false,
                    port: 3000,
                },
                TopologyNode {
                    name: "server:data".to_string(),
                    purpose: "OpenAI-compatible gateway".to_string(),
                    hot_path: true,
                    port: self.config.data_port,
                },
                TopologyNode {
                    name: "server:admin".to_string(),
                    purpose: "Scheduler, tenancy, and observability".to_string(),
                    hot_path: false,
                    port: self.config.admin_port,
                },
                TopologyNode {
                    name: "browser-assist".to_string(),
                    purpose: "Challenge recovery and login sidecar".to_string(),
                    hot_path: false,
                    port: 8090,
                },
            ],
            cache_metrics,
            accounts: account_summaries,
            leases: leases.clone(),
            cf_incidents,
            browser_tasks: Vec::new(),
            users: users.clone(),
            request_logs,
            billing,
            model_catalog,
            counts: DashboardCounts {
                tenants: tenants.len(),
                accounts: accounts.len(),
                users: users.len(),
                active_leases: leases.len(),
                warp_accounts: route_states
                    .iter()
                    .filter(|route_state| route_state.route_mode == RouteMode::Warp)
                    .count(),
                browser_tasks: 0,
            },
        }
    }

    pub async fn list_tenants(&self) -> Vec<Tenant> {
        let mut tenants = self
            .runtime
            .tenants
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        tenants.sort_by(|left, right| left.slug.cmp(&right.slug));
        tenants
    }

    pub async fn list_accounts(&self) -> Vec<AccountSummary> {
        self.account_summaries().await
    }

    pub async fn list_egress_slots(&self) -> Vec<EgressSlot> {
        vec![
            EgressSlot {
                id: "direct".to_string(),
                route_mode: RouteMode::Direct,
                configured: self.config.direct_proxy_url.is_some()
                    || self.config.browser_assist_direct_proxy_url.is_some(),
                upstream_proxy_url_preview: self
                    .config
                    .direct_proxy_url
                    .as_deref()
                    .map(mask_endpoint),
                browser_proxy_url_preview: self
                    .config
                    .browser_assist_direct_proxy_url
                    .as_deref()
                    .map(mask_endpoint),
            },
            EgressSlot {
                id: "warp".to_string(),
                route_mode: RouteMode::Warp,
                configured: self.config.warp_proxy_url.is_some()
                    || self.config.browser_assist_warp_proxy_url.is_some(),
                upstream_proxy_url_preview: self
                    .config
                    .warp_proxy_url
                    .as_deref()
                    .map(mask_endpoint),
                browser_proxy_url_preview: self
                    .config
                    .browser_assist_warp_proxy_url
                    .as_deref()
                    .map(mask_endpoint),
            },
        ]
    }

    pub async fn list_leases(&self) -> Vec<CliLease> {
        let mut leases = self
            .runtime
            .leases
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        leases.sort_by(|left, right| right.last_used_at.cmp(&left.last_used_at));
        leases
    }

    pub async fn list_cf_incidents(&self) -> Vec<CfIncident> {
        self.runtime.cf_incidents.read().await.clone()
    }

    pub async fn cache_metrics(&self) -> CacheMetrics {
        self.runtime.cache_metrics.read().await.clone()
    }

    pub async fn list_api_keys(&self) -> Vec<GatewayApiKeyView> {
        let mut api_keys = self
            .runtime
            .api_keys
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        api_keys.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        api_keys
            .into_iter()
            .map(|api_key| GatewayApiKeyView {
                id: api_key.id,
                tenant_id: api_key.tenant_id,
                name: api_key.name,
                email: api_key.email,
                role: api_key.role,
                token_preview: mask_token(&api_key.token),
                default_model: api_key.default_model,
                reasoning_effort: api_key.reasoning_effort,
                force_model_override: api_key.force_model_override,
                force_reasoning_effort: api_key.force_reasoning_effort,
                created_at: api_key.created_at,
                updated_at: api_key.updated_at,
            })
            .collect()
    }

    pub async fn list_request_logs(&self) -> Vec<RequestLogEntry> {
        self.runtime.request_logs.read().await.clone()
    }

    pub async fn list_gateway_users(&self) -> Vec<GatewayUserView> {
        let logs = self.list_request_logs().await;
        self.gateway_user_views(&logs).await
    }

    pub async fn create_tenant(&self, request: CreateTenantRequest) -> Tenant {
        let tenant = Tenant {
            id: Uuid::new_v4(),
            slug: request.slug,
            name: request.name,
            created_at: Utc::now(),
        };
        self.runtime
            .tenants
            .write()
            .await
            .insert(tenant.id, tenant.clone());
        self.enqueue(PersistenceMessage::TenantUpsert(tenant.clone()))
            .await;
        tenant
    }

    async fn ensure_default_tenant(&self) -> Uuid {
        if let Some(existing_id) = self
            .runtime
            .tenants
            .read()
            .await
            .values()
            .min_by_key(|tenant| tenant.created_at)
            .map(|tenant| tenant.id)
        {
            return existing_id;
        }

        let (tenant, inserted) = {
            let mut tenants = self.runtime.tenants.write().await;
            if let Some(existing) = tenants
                .values()
                .min_by_key(|tenant| tenant.created_at)
                .cloned()
            {
                (existing, false)
            } else {
                let tenant = Tenant {
                    id: Uuid::new_v4(),
                    slug: "default".to_string(),
                    name: "默认租户".to_string(),
                    created_at: Utc::now(),
                };
                tenants.insert(tenant.id, tenant.clone());
                (tenant, true)
            }
        };

        if inserted {
            self.enqueue(PersistenceMessage::TenantUpsert(tenant.clone()))
                .await;
        }

        tenant.id
    }

    async fn resolve_openai_login_tenant(&self, requested: Option<Uuid>) -> Result<Uuid, String> {
        if let Some(tenant_id) = requested {
            let exists = self.runtime.tenants.read().await.contains_key(&tenant_id);
            if !exists {
                return Err("指定租户不存在。".to_string());
            }
            return Ok(tenant_id);
        }

        Ok(self.ensure_default_tenant().await)
    }

    async fn resolve_gateway_user_tenant(&self, requested: Option<Uuid>) -> Result<Uuid, String> {
        self.resolve_openai_login_tenant(requested).await
    }

    pub async fn create_api_key(
        &self,
        request: CreateGatewayApiKeyRequest,
    ) -> Option<CreatedGatewayApiKey> {
        let tenant_exists = self
            .runtime
            .tenants
            .read()
            .await
            .contains_key(&request.tenant_id);
        if !tenant_exists {
            return None;
        }

        let api_key = GatewayApiKey {
            id: Uuid::new_v4(),
            tenant_id: request.tenant_id,
            name: request.name,
            email: String::new(),
            role: GatewayUserRole::Viewer,
            token: format!("cmgr_{}", Uuid::new_v4().simple()),
            default_model: None,
            reasoning_effort: None,
            force_model_override: false,
            force_reasoning_effort: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.runtime
            .api_keys
            .write()
            .await
            .insert(api_key.token.clone(), api_key.clone());
        self.enqueue(PersistenceMessage::ApiKeyUpsert(api_key.clone()))
            .await;
        Some(CreatedGatewayApiKey {
            id: api_key.id,
            tenant_id: api_key.tenant_id,
            name: api_key.name,
            email: api_key.email,
            role: api_key.role,
            token: api_key.token,
            default_model: api_key.default_model,
            reasoning_effort: api_key.reasoning_effort,
            force_model_override: api_key.force_model_override,
            force_reasoning_effort: api_key.force_reasoning_effort,
            created_at: api_key.created_at,
            updated_at: api_key.updated_at,
        })
    }

    pub async fn create_gateway_user(
        &self,
        request: CreateGatewayUserRequest,
    ) -> Result<CreatedGatewayUser, String> {
        let tenant_id = self.resolve_gateway_user_tenant(request.tenant_id).await?;
        let name = request.name.trim().to_string();
        let email = request.email.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err("用户名不能为空。".to_string());
        }
        if email.is_empty() {
            return Err("邮箱不能为空。".to_string());
        }
        if !email.contains('@') {
            return Err("邮箱格式无效。".to_string());
        }

        {
            let exists = self.runtime.api_keys.read().await.values().any(|api_key| {
                api_key.email.eq_ignore_ascii_case(&email) && api_key.tenant_id == tenant_id
            });
            if exists {
                return Err("该邮箱已经存在。".to_string());
            }
        }

        let now = Utc::now();
        let api_key = GatewayApiKey {
            id: Uuid::new_v4(),
            tenant_id,
            name,
            email,
            role: request.role,
            token: format!("cmgr_{}", Uuid::new_v4().simple()),
            default_model: request
                .default_model
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            reasoning_effort: normalize_reasoning_effort(request.reasoning_effort.as_deref()),
            force_model_override: request.force_model_override.unwrap_or(false),
            force_reasoning_effort: request.force_reasoning_effort.unwrap_or(false),
            created_at: now,
            updated_at: now,
        };

        self.runtime
            .api_keys
            .write()
            .await
            .insert(api_key.token.clone(), api_key.clone());
        self.enqueue(PersistenceMessage::ApiKeyUpsert(api_key.clone()))
            .await;

        let logs = self.list_request_logs().await;
        let user = self
            .gateway_user_views(&logs)
            .await
            .into_iter()
            .find(|item| item.id == api_key.id)
            .unwrap_or_else(|| gateway_user_view(&api_key, 0, 0.0, None));

        Ok(CreatedGatewayUser {
            user,
            token: api_key.token,
        })
    }

    pub async fn update_gateway_user(
        &self,
        user_id: Uuid,
        request: UpdateGatewayUserRequest,
    ) -> Result<GatewayUserView, String> {
        let duplicate_email = request
            .email
            .as_ref()
            .map(|value| value.trim().to_ascii_lowercase());
        if let Some(email) = duplicate_email.as_deref() {
            if email.is_empty() || !email.contains('@') {
                return Err("邮箱格式无效。".to_string());
            }
            let api_keys = self.runtime.api_keys.read().await;
            let Some(current) = api_keys.values().find(|api_key| api_key.id == user_id) else {
                return Err("用户不存在。".to_string());
            };
            let duplicate = api_keys.values().any(|api_key| {
                api_key.id != user_id
                    && api_key.tenant_id == current.tenant_id
                    && api_key.email.eq_ignore_ascii_case(email)
            });
            if duplicate {
                return Err("该邮箱已经存在。".to_string());
            }
        }

        let updated = {
            let mut api_keys = self.runtime.api_keys.write().await;
            let Some(existing) = api_keys.values_mut().find(|api_key| api_key.id == user_id)
            else {
                return Err("用户不存在。".to_string());
            };

            if let Some(name) = request.name {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err("用户名不能为空。".to_string());
                }
                existing.name = name;
            }
            if let Some(email) = duplicate_email {
                existing.email = email;
            }
            if let Some(role) = request.role {
                existing.role = role;
            }
            if let Some(default_model) = request.default_model {
                existing.default_model = Some(default_model.trim().to_string())
                    .filter(|value| !value.is_empty());
            }
            if request.reasoning_effort.is_some() {
                existing.reasoning_effort =
                    normalize_reasoning_effort(request.reasoning_effort.as_deref());
            }
            if let Some(force_model_override) = request.force_model_override {
                existing.force_model_override = force_model_override;
            }
            if let Some(force_reasoning_effort) = request.force_reasoning_effort {
                existing.force_reasoning_effort = force_reasoning_effort;
            }
            existing.updated_at = Utc::now();
            existing.clone()
        };

        self.enqueue(PersistenceMessage::ApiKeyUpsert(updated.clone()))
            .await;

        let logs = self.list_request_logs().await;
        self.gateway_user_views(&logs)
            .await
            .into_iter()
            .find(|item| item.id == updated.id)
            .ok_or_else(|| "用户不存在。".to_string())
    }

    pub async fn start_openai_login(
        &self,
        request: OpenAiLoginStartRequest,
    ) -> Result<OpenAiLoginStartResponse, String> {
        let OpenAiLoginStartRequest {
            tenant_id,
            label,
            note,
            redirect_uri,
            models,
            base_url,
        } = request;
        let tenant_id = self.resolve_openai_login_tenant(tenant_id).await?;
        let redirect_uri = reqwest::Url::parse(&redirect_uri)
            .map_err(|error| format!("redirectUri 无效: {error}"))?
            .to_string();
        let pkce = openai_auth::generate_pkce();
        let login_id = openai_auth::generate_state();
        let auth_url =
            openai_auth::build_authorize_url(&redirect_uri, &pkce.code_challenge, &login_id)?;
        let now = Utc::now();
        let session = OpenAiLoginSessionState {
            login_id: login_id.clone(),
            tenant_id,
            label: label.filter(|value| !value.trim().is_empty()),
            note: note.filter(|value| !value.trim().is_empty()),
            redirect_uri: redirect_uri.clone(),
            auth_url: auth_url.clone(),
            code_verifier: pkce.code_verifier,
            status: "pending".to_string(),
            error: None,
            imported_account_id: None,
            imported_account_label: None,
            models: models.unwrap_or_else(default_oauth_models),
            base_url: base_url
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(default_oauth_base_url),
            created_at: now,
            updated_at: now,
        };
        self.runtime
            .openai_login_sessions
            .write()
            .await
            .insert(login_id.clone(), session);

        Ok(OpenAiLoginStartResponse {
            login_id,
            auth_url,
            redirect_uri,
        })
    }

    pub async fn openai_login_status(&self, login_id: &str) -> Option<OpenAiLoginSessionView> {
        self.runtime
            .openai_login_sessions
            .read()
            .await
            .get(login_id)
            .cloned()
            .map(|session| session.view())
    }

    pub async fn complete_openai_login(
        &self,
        request: OpenAiLoginCompleteRequest,
    ) -> Result<UpstreamAccount, String> {
        let session = self
            .runtime
            .openai_login_sessions
            .read()
            .await
            .get(&request.state)
            .cloned()
            .ok_or_else(|| "未知登录会话。".to_string())?;
        if session.status == "success" {
            if let Some(account_id) = session.imported_account_id.as_deref() {
                if let Some(existing) = self.runtime.accounts.read().await.get(account_id).cloned()
                {
                    return Ok(existing);
                }
            }
        }
        let redirect_uri = request
            .redirect_uri
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| session.redirect_uri.clone());

        {
            let mut sessions = self.runtime.openai_login_sessions.write().await;
            if let Some(active) = sessions.get_mut(&request.state) {
                active.status = "exchanging".to_string();
                active.error = None;
                active.updated_at = Utc::now();
            }
        }

        let result = async {
            let tokens = openai_auth::exchange_code_for_tokens(
                &redirect_uri,
                &session.code_verifier,
                &request.code,
            )
            .await?;
            let claims = openai_auth::parse_id_token_claims(&tokens.id_token)?;
            let account_label = session
                .label
                .clone()
                .or_else(|| claims.email.clone())
                .or_else(|| session.note.clone())
                .unwrap_or_else(|| format!("OpenAI {}", claims.sub));
            let chatgpt_account_id = openai_auth::extract_chatgpt_account_id(&tokens.id_token)
                .or_else(|| claims.auth.clone().and_then(|auth| auth.chatgpt_account_id))
                .or_else(|| openai_auth::extract_chatgpt_account_id(&tokens.access_token))
                .or(claims.workspace_id.clone());

            let account = self
                .import_account(ImportAccountRequest {
                    tenant_id: session.tenant_id,
                    label: account_label,
                    models: session.models.clone(),
                    quota_headroom: Some(0.8),
                    quota_headroom_5h: Some(0.8),
                    quota_headroom_7d: Some(0.8),
                    health_score: Some(0.9),
                    egress_stability: Some(0.85),
                    base_url: Some(session.base_url.clone()),
                    bearer_token: Some(tokens.access_token),
                    chatgpt_account_id,
                    extra_headers: None,
                })
                .await;
            Ok::<UpstreamAccount, String>(account)
        }
        .await;

        let mut sessions = self.runtime.openai_login_sessions.write().await;
        if let Some(active) = sessions.get_mut(&request.state) {
            active.updated_at = Utc::now();
            match &result {
                Ok(account) => {
                    active.status = "success".to_string();
                    active.error = None;
                    active.imported_account_id = Some(account.id.clone());
                    active.imported_account_label = Some(account.label.clone());
                }
                Err(error) => {
                    active.status = "failed".to_string();
                    active.error = Some(error.clone());
                }
            }
        }

        result
    }

    pub async fn import_account(&self, request: ImportAccountRequest) -> UpstreamAccount {
        let credential = build_credential_template(&request);
        let quota_headroom = request.quota_headroom.unwrap_or(0.7);
        let account = UpstreamAccount {
            id: format!("acc_{}", Uuid::new_v4().simple()),
            tenant_id: request.tenant_id,
            label: request.label,
            models: request.models,
            current_mode: RouteMode::Direct,
            signals: SchedulingSignals {
                quota_headroom,
                quota_headroom_5h: request.quota_headroom_5h.unwrap_or(quota_headroom),
                quota_headroom_7d: request.quota_headroom_7d.unwrap_or(quota_headroom),
                health_score: request.health_score.unwrap_or(0.85),
                egress_stability: request.egress_stability.unwrap_or(0.8),
                fairness_bias: 0.7,
                inflight: 0,
                capacity: 4,
            },
            created_at: Utc::now(),
        };
        let route_state = AccountRouteState {
            account_id: account.id.clone(),
            route_mode: RouteMode::Direct,
            direct_cf_streak: 0,
            warp_cf_streak: 0,
            cooldown_level: 0,
            cooldown_until: None,
            warp_entered_at: None,
            last_cf_at: None,
            success_streak: 0,
            last_success_at: None,
        };
        self.runtime
            .route_states
            .write()
            .await
            .insert(account.id.clone(), route_state.clone());
        self.runtime
            .accounts
            .write()
            .await
            .insert(account.id.clone(), account.clone());

        let credential = credential.map(|credential| UpstreamCredential {
            account_id: account.id.clone(),
            ..credential
        });
        if let Some(credential) = credential.clone() {
            self.runtime
                .credentials
                .write()
                .await
                .insert(account.id.clone(), credential);
        }

        self.enqueue(PersistenceMessage::AccountUpsert(account.clone()))
            .await;
        if let Some(credential) = credential {
            self.enqueue(PersistenceMessage::CredentialUpsert(credential))
                .await;
        }
        self.enqueue(PersistenceMessage::RouteStateUpsert(route_state))
            .await;
        account
    }

    pub async fn credential_for_account(&self, account_id: &str) -> Option<UpstreamCredential> {
        self.runtime
            .credentials
            .read()
            .await
            .get(account_id)
            .cloned()
    }

    pub async fn near_quota_guard_enabled(&self, account_id: &str) -> bool {
        self.runtime
            .accounts
            .read()
            .await
            .get(account_id)
            .is_some_and(|account| account.signals.near_quota_guard_enabled())
    }

    pub async fn auth_context_for_bearer(&self, bearer_token: &str) -> Option<GatewayAuthContext> {
        let api_key = self
            .runtime
            .api_keys
            .read()
            .await
            .get(bearer_token)
            .cloned()?;
        let tenant = self
            .runtime
            .tenants
            .read()
            .await
            .get(&api_key.tenant_id)
            .cloned()?;
        Some(GatewayAuthContext { tenant, api_key })
    }

    pub async fn tenant_for_bearer(&self, bearer_token: &str) -> Option<Tenant> {
        self.auth_context_for_bearer(bearer_token)
            .await
            .map(|context| context.tenant)
    }

    pub async fn refresh_account_models(&self) -> Result<Vec<AccountSummary>, String> {
        let accounts = self
            .runtime
            .accounts
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let credentials = self.runtime.credentials.read().await.clone();
        let mut updates = Vec::new();

        for account in accounts {
            let Some(credential) = credentials.get(account.id.as_str()).cloned() else {
                continue;
            };
            match self.upstream.list_models(&credential, account.current_mode).await {
                Ok(models) if !models.is_empty() => {
                    let mut normalized = models
                        .into_iter()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>();
                    normalized.sort();
                    normalized.dedup();
                    if normalized != account.models {
                        updates.push((account.id.clone(), normalized));
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    warn!(account_id = %account.id, %error, "failed to refresh upstream model catalog");
                }
            }
        }

        if !updates.is_empty() {
            let snapshots = {
                let mut accounts = self.runtime.accounts.write().await;
                let mut snapshots = Vec::new();
                for (account_id, models) in updates {
                    if let Some(account) = accounts.get_mut(account_id.as_str()) {
                        account.models = models;
                        snapshots.push(account.clone());
                    }
                }
                snapshots
            };
            self.enqueue_many(
                snapshots
                    .into_iter()
                    .map(PersistenceMessage::AccountUpsert)
                    .collect(),
            )
            .await;
        }

        Ok(self.account_summaries().await)
    }

    pub async fn record_request_log(&self, entry: RequestLogEntry) {
        {
            let mut logs = self.runtime.request_logs.write().await;
            logs.insert(0, entry.clone());
            logs.truncate(512);
        }
        self.enqueue(PersistenceMessage::RequestLogInsert(entry)).await;
    }

    fn billing_summary(&self, logs: &[RequestLogEntry]) -> BillingSummary {
        logs.iter().fold(
            BillingSummary {
                total_spend_usd: 0.0,
                total_requests: logs.len(),
                total_input_tokens: 0,
                total_cached_input_tokens: 0,
                total_output_tokens: 0,
                total_tokens: 0,
                priced_requests: 0,
            },
            |mut summary, log| {
                summary.total_input_tokens += log.usage.input_tokens;
                summary.total_cached_input_tokens += log.usage.cached_input_tokens;
                summary.total_output_tokens += log.usage.output_tokens;
                summary.total_tokens += log.usage.total_tokens;
                if let Some(cost) = log.estimated_cost_usd {
                    summary.total_spend_usd += cost;
                    summary.priced_requests += 1;
                }
                summary
            },
        )
    }

    async fn gateway_user_views(&self, logs: &[RequestLogEntry]) -> Vec<GatewayUserView> {
        let mut api_keys = self
            .runtime
            .api_keys
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        api_keys.sort_by(|left, right| left.created_at.cmp(&right.created_at));

        api_keys
            .into_iter()
            .map(|api_key| {
                let request_count = logs
                    .iter()
                    .filter(|log| log.api_key_id == api_key.id)
                    .count();
                let estimated_spend_usd = logs
                    .iter()
                    .filter(|log| log.api_key_id == api_key.id)
                    .filter_map(|log| log.estimated_cost_usd)
                    .sum::<f64>();
                let last_used_at = logs
                    .iter()
                    .filter(|log| log.api_key_id == api_key.id)
                    .map(|log| log.created_at)
                    .max();
                gateway_user_view(&api_key, request_count, estimated_spend_usd, last_used_at)
            })
            .collect()
    }

    pub async fn record_route_event(
        &self,
        account_id: &str,
        event: RouteEventRequest,
    ) -> Option<AccountRouteState> {
        let now = Utc::now();
        let (state_snapshot, severity, should_recover, should_failover) = {
            let mut route_states = self.runtime.route_states.write().await;
            let state = route_states.get_mut(account_id)?;
            reconcile_route_mode(state, now);

            match event.kind.as_str() {
                "cf_hit" => {
                    let outcome = register_cf_hit(state, event.mode, now);
                    let severity = if outcome.switched_to_warp {
                        "direct-escalation"
                    } else {
                        "cooldown"
                    }
                    .to_string();
                    (
                        state.clone(),
                        Some(severity),
                        true,
                        outcome.failover_required,
                    )
                }
                "success" => {
                    register_success(state, now);
                    (state.clone(), None, false, false)
                }
                _ => return Some(state.clone()),
            }
        };

        let account_snapshot = {
            let mut accounts = self.runtime.accounts.write().await;
            let account = accounts.get_mut(account_id)?;
            account.current_mode = state_snapshot.route_mode;
            account.clone()
        };

        let severity_label = severity.clone();
        let incident = severity.map(|severity| {
            let account_label = account_snapshot.label.clone();
            CfIncident {
                id: Uuid::new_v4().to_string(),
                account_id: account_id.to_string(),
                account_label,
                route_mode: state_snapshot.route_mode,
                severity,
                happened_at: now,
                cooldown_level: state_snapshot.cooldown_level,
            }
        });

        if let Some(incident) = incident.clone() {
            let mut incidents = self.runtime.cf_incidents.write().await;
            incidents.insert(0, incident.clone());
            incidents.truncate(128);
        }

        let mut messages = vec![
            PersistenceMessage::AccountUpsert(account_snapshot),
            PersistenceMessage::RouteStateUpsert(state_snapshot.clone()),
        ];
        if let Some(incident) = incident {
            messages.push(PersistenceMessage::IncidentInsert(incident));
        }
        if should_failover {
            let evicted_leases = self.evict_leases_for_account(account_id).await;
            messages.extend(
                evicted_leases
                    .iter()
                    .map(|lease| PersistenceMessage::LeaseDelete(lease.principal_id.clone())),
            );
        }
        self.enqueue_many(messages).await;
        if should_recover {
            let credential = self.credential_for_account(account_id).await;
            let provider = browser_task_provider_for_credential(credential.as_ref());
            let login_url =
                browser_task_login_url_for_credential(credential.as_ref(), provider.as_deref());
            let notes = Some(format!(
                "routeMode={} cooldownLevel={} severity={}",
                state_snapshot.route_mode.as_str(),
                state_snapshot.cooldown_level,
                severity_label.unwrap_or_else(|| "unknown".to_string())
            ));
            browser_assist::spawn_recover(
                self.config.browser_assist_url.clone(),
                browser_assist::BrowserTaskPayload {
                    account_id: Some(account_id.to_string()),
                    notes,
                    login_url,
                    headless: Some(true),
                    provider,
                    email: None,
                    password: None,
                    otp_code: None,
                    route_mode: Some(state_snapshot.route_mode),
                },
            );
        }

        Some(state_snapshot)
    }

    pub async fn failover_account(
        &self,
        account_id: &str,
        severity: &str,
        cooldown_seconds: i64,
        should_recover: bool,
    ) -> Option<AccountRouteState> {
        let now = Utc::now();
        let state_snapshot = {
            let mut route_states = self.runtime.route_states.write().await;
            let state = route_states.get_mut(account_id)?;
            state.success_streak = 0;
            state.cooldown_until = Some(now + ChronoDuration::seconds(cooldown_seconds));
            state.cooldown_level = state.cooldown_level.max(1);
            state.last_cf_at = Some(now);
            state.clone()
        };

        let account_snapshot = {
            let mut accounts = self.runtime.accounts.write().await;
            let account = accounts.get_mut(account_id)?;
            account.current_mode = state_snapshot.route_mode;
            account.clone()
        };

        let incident = CfIncident {
            id: Uuid::new_v4().to_string(),
            account_id: account_id.to_string(),
            account_label: account_snapshot.label.clone(),
            route_mode: state_snapshot.route_mode,
            severity: severity.to_string(),
            happened_at: now,
            cooldown_level: state_snapshot.cooldown_level,
        };

        {
            let mut incidents = self.runtime.cf_incidents.write().await;
            incidents.insert(0, incident.clone());
            incidents.truncate(128);
        }

        let evicted_leases = self.evict_leases_for_account(account_id).await;
        let mut messages = vec![
            PersistenceMessage::AccountUpsert(account_snapshot),
            PersistenceMessage::RouteStateUpsert(state_snapshot.clone()),
            PersistenceMessage::IncidentInsert(incident),
        ];
        messages.extend(
            evicted_leases
                .iter()
                .map(|lease| PersistenceMessage::LeaseDelete(lease.principal_id.clone())),
        );
        self.enqueue_many(messages).await;

        if should_recover {
            let credential = self.credential_for_account(account_id).await;
            let provider = browser_task_provider_for_credential(credential.as_ref());
            let login_url =
                browser_task_login_url_for_credential(credential.as_ref(), provider.as_deref());
            let notes = Some(format!(
                "routeMode={} cooldownLevel={} severity={}",
                state_snapshot.route_mode.as_str(),
                state_snapshot.cooldown_level,
                severity
            ));
            browser_assist::spawn_recover(
                self.config.browser_assist_url.clone(),
                browser_assist::BrowserTaskPayload {
                    account_id: Some(account_id.to_string()),
                    notes,
                    login_url,
                    headless: Some(true),
                    provider,
                    email: None,
                    password: None,
                    otp_code: None,
                    route_mode: Some(state_snapshot.route_mode),
                },
            );
        }

        Some(state_snapshot)
    }

    pub async fn resolve_lease(
        &self,
        request: LeaseSelectionRequest,
    ) -> Option<(CliLease, ReplayPack, WarmupDecision)> {
        let now = Utc::now();
        let principal_id = request.principal_id.clone();

        let mut changed_route_states = Vec::new();
        {
            let mut route_states = self.runtime.route_states.write().await;
            for route_state in route_states.values_mut() {
                let previous_mode = route_state.route_mode;
                reconcile_route_mode(route_state, now);
                if previous_mode != route_state.route_mode {
                    changed_route_states.push(route_state.clone());
                }
            }
        }

        if !changed_route_states.is_empty() {
            let account_ids = changed_route_states
                .iter()
                .map(|route_state| route_state.account_id.clone())
                .collect::<Vec<_>>();
            let mut updated_accounts = Vec::new();
            {
                let mut accounts = self.runtime.accounts.write().await;
                for route_state in &changed_route_states {
                    if let Some(account) = accounts.get_mut(&route_state.account_id) {
                        account.current_mode = route_state.route_mode;
                        updated_accounts.push(account.clone());
                    }
                }
            }
            self.enqueue_many(
                changed_route_states
                    .into_iter()
                    .map(PersistenceMessage::RouteStateUpsert)
                    .chain(
                        updated_accounts
                            .into_iter()
                            .map(PersistenceMessage::AccountUpsert),
                    )
                    .collect::<Vec<_>>(),
            )
            .await;
            info!(
                changed_accounts = account_ids.len(),
                "reconciled route-mode drift before lease selection"
            );
        }

        let accounts = self
            .runtime
            .accounts
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let route_states = self.runtime.route_states.read().await.clone();
        let credentialed_accounts = self
            .runtime
            .credentials
            .read()
            .await
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let mut candidate_accounts = accounts
            .iter()
            .filter(|account| {
                account.tenant_id == request.tenant_id
                    && credentialed_accounts.contains(account.id.as_str())
                    && account.models.iter().any(|model| model == &request.model)
                    && route_states
                        .get(account.id.as_str())
                        .is_some_and(|route_state| !is_in_cooldown(route_state, now))
            })
            .cloned()
            .collect::<Vec<_>>();

        candidate_accounts.sort_by(|left, right| left.label.cmp(&right.label));

        let existing_lease = self.runtime.leases.read().await.get(&principal_id).cloned();
        if let Some(existing) = existing_lease.as_ref() {
            if let (Some(account), Some(route_state)) = (
                accounts
                    .iter()
                    .find(|account| account.id == existing.account_id),
                route_states.get(existing.account_id.as_str()),
            ) {
                if credentialed_accounts.contains(account.id.as_str())
                    && should_reuse_lease(existing, account, route_state)
                    && account.models.contains(&request.model)
                {
                    let lease_snapshot = {
                        let mut leases = self.runtime.leases.write().await;
                        let lease = leases.get_mut(&principal_id)?;
                        lease.last_used_at = now;
                        lease.active_subagents = request.subagent_count;
                        lease.route_mode = route_state.route_mode;
                        lease.clone()
                    };
                    let replay = compile_replay_pack(
                        &principal_id,
                        &request.model,
                        existing.generation,
                        &serde_json::json!({"principal": principal_id, "model": request.model}),
                    );
                    let warmup = evaluate_prefix_warmup(
                        3,
                        replay.static_prefix_tokens,
                        0.75,
                        replay.live_tail_tokens,
                        false,
                    );
                    self.enqueue(PersistenceMessage::LeaseUpsert(lease_snapshot.clone()))
                        .await;
                    return Some((lease_snapshot, replay, warmup));
                }
            }
        }

        if candidate_accounts.is_empty() {
            return None;
        }

        let dual_candidates =
            select_dual_candidates(&principal_id, &request.model, &candidate_accounts);
        let selected = dual_candidates
            .iter()
            .filter_map(|account| {
                let route_state = route_states.get(account.id.as_str())?;
                Some((
                    (*account).clone(),
                    score_candidate(account, route_state, existing_lease.as_ref()),
                ))
            })
            .max_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(account, _)| account)
            .or_else(|| candidate_accounts.first().cloned())?;

        let generation = existing_lease
            .as_ref()
            .map(|lease| lease.generation + 1)
            .unwrap_or(1);
        let selected_route_mode = route_states
            .get(selected.id.as_str())
            .map(|route_state| route_state.route_mode)
            .unwrap_or(selected.current_mode);
        let lease = CliLease {
            principal_id: principal_id.clone(),
            tenant_id: request.tenant_id,
            account_id: selected.id.clone(),
            account_label: selected.label.clone(),
            model: request.model.clone(),
            reasoning_effort: request.reasoning_effort.clone(),
            route_mode: selected_route_mode,
            generation,
            active_subagents: request.subagent_count,
            created_at: now,
            last_used_at: now,
        };
        self.runtime
            .leases
            .write()
            .await
            .insert(principal_id.clone(), lease.clone());

        let replay = compile_replay_pack(
            &principal_id,
            &request.model,
            generation,
            &serde_json::json!({"principal": principal_id, "model": request.model}),
        );
        let warmup = evaluate_prefix_warmup(
            4,
            replay.static_prefix_tokens + replay.workflow_tokens,
            0.75,
            replay.live_tail_tokens,
            false,
        );

        let metrics_snapshot = {
            let mut metrics = self.runtime.cache_metrics.write().await;
            metrics.cached_tokens += replay.static_prefix_tokens;
            metrics.replay_tokens += replay.total_tokens;
            metrics.prefix_hit_ratio = ((metrics.prefix_hit_ratio * 9.0) + 0.88) / 10.0;
            if warmup.should_warm {
                metrics.warmup_roi =
                    ((metrics.warmup_roi * 4.0) + warmup.expected_saving.max(1.0)) / 5.0;
            }
            metrics.clone()
        };

        self.enqueue_many(vec![
            PersistenceMessage::LeaseUpsert(lease.clone()),
            PersistenceMessage::CacheMetricsUpsert(metrics_snapshot),
        ])
        .await;

        Some((lease, replay, warmup))
    }

    pub async fn replay_context_for(&self, principal_id: &str, generation: u32) -> Option<String> {
        if generation <= 1 {
            return None;
        }
        let contexts = self.runtime.conversation_contexts.read().await;
        let context = contexts.get(principal_id)?;
        if context.turns.is_empty() {
            return None;
        }

        let mut block = String::new();
        block.push_str("[cmgr replay context]\n");
        block.push_str("Replay only exists to stabilize account failover.\n");
        block.push_str(&format!("principal={}\n", context.principal_id));
        block.push_str(&format!("model={}\n", context.model));
        block.push_str(&format!("generation={generation}\n"));
        if !context.workflow_spine.is_empty() {
            block.push_str("workflow=\n");
            block.push_str(&context.workflow_spine);
            block.push('\n');
        }
        block.push_str("recent_turns=\n");
        for (index, turn) in context.turns.iter().rev().take(6).rev().enumerate() {
            block.push_str(&format!(
                "{}. g{} user: {}\n",
                index + 1,
                turn.generation,
                turn.request_summary
            ));
            if let Some(response_summary) = turn.response_summary.as_deref() {
                block.push_str(&format!("   assistant: {}\n", response_summary));
            }
        }
        Some(block)
    }

    pub async fn record_context_input(
        &self,
        principal_id: &str,
        model: &str,
        generation: u32,
        request_summary: String,
    ) {
        let snapshot = {
            let mut contexts = self.runtime.conversation_contexts.write().await;
            let context = contexts
                .entry(principal_id.to_string())
                .or_insert_with(|| ConversationContext {
                    principal_id: principal_id.to_string(),
                    model: model.to_string(),
                    workflow_spine: format!(
                        "Keep the active task coherent across account failover. Preserve exact model={} and lease affinity.",
                        model
                    ),
                    turns: Vec::new(),
                    updated_at: Some(Utc::now()),
                });
            context.model = model.to_string();
            context.updated_at = Some(Utc::now());
            context.turns.push(ContextTurn {
                generation,
                request_summary,
                response_summary: None,
                created_at: Utc::now(),
            });
            if context.turns.len() > 12 {
                let drain = context.turns.len() - 12;
                context.turns.drain(0..drain);
            }
            context.clone()
        };
        self.enqueue(PersistenceMessage::ConversationContextUpsert(snapshot))
            .await;
    }

    pub async fn record_context_output(&self, principal_id: &str, response_summary: String) {
        let snapshot = {
            let mut contexts = self.runtime.conversation_contexts.write().await;
            let Some(context) = contexts.get_mut(principal_id) else {
                return;
            };
            if let Some(turn) = context.turns.last_mut() {
                turn.response_summary = Some(response_summary);
                turn.created_at = Utc::now();
            }
            context.updated_at = Some(Utc::now());
            context.clone()
        };
        self.enqueue(PersistenceMessage::ConversationContextUpsert(snapshot))
            .await;
    }

    async fn enqueue(&self, message: PersistenceMessage) {
        let _ = self.writer_tx.send(message.clone()).await;
        if let Some(bus_tx) = self.bus_tx.as_ref() {
            let _ = bus_tx.send(message).await;
        }
    }

    async fn enqueue_many(&self, messages: Vec<PersistenceMessage>) {
        for message in messages {
            self.enqueue(message).await;
        }
    }

    async fn evict_leases_for_account(&self, account_id: &str) -> Vec<CliLease> {
        let mut leases = self.runtime.leases.write().await;
        let principals = leases
            .iter()
            .filter(|(_, lease)| lease.account_id == account_id)
            .map(|(principal_id, _)| principal_id.clone())
            .collect::<Vec<_>>();
        principals
            .into_iter()
            .filter_map(|principal_id| leases.remove(&principal_id))
            .collect()
    }

    async fn account_summaries(&self) -> Vec<AccountSummary> {
        let mut accounts = self
            .runtime
            .accounts
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        accounts.sort_by(|left, right| left.label.cmp(&right.label));
        let route_states = self.runtime.route_states.read().await.clone();
        let credentials = self.runtime.credentials.read().await.clone();

        accounts
            .into_iter()
            .map(|account| {
                let route_state = route_states.get(account.id.as_str()).cloned();
                let credential = credentials.get(account.id.as_str()).cloned();
                let resolved_route_mode = route_state
                    .as_ref()
                    .map(|route_state| route_state.route_mode)
                    .unwrap_or(account.current_mode);
                AccountSummary {
                    id: account.id.clone(),
                    tenant_id: account.tenant_id,
                    label: account.label.clone(),
                    models: account.models.clone(),
                    current_mode: account.current_mode,
                    route_mode: resolved_route_mode,
                    cooldown_level: route_state
                        .as_ref()
                        .map(|route_state| route_state.cooldown_level)
                        .unwrap_or_default(),
                    cooldown_until: route_state
                        .as_ref()
                        .and_then(|route_state| route_state.cooldown_until),
                    direct_cf_streak: route_state
                        .as_ref()
                        .map(|route_state| route_state.direct_cf_streak)
                        .unwrap_or_default(),
                    warp_cf_streak: route_state
                        .as_ref()
                        .map(|route_state| route_state.warp_cf_streak)
                        .unwrap_or_default(),
                    success_streak: route_state
                        .as_ref()
                        .map(|route_state| route_state.success_streak)
                        .unwrap_or_default(),
                    quota_headroom: account.signals.effective_quota_headroom(),
                    quota_headroom_5h: account.signals.quota_headroom_5h,
                    quota_headroom_7d: account.signals.quota_headroom_7d,
                    near_quota_guard_enabled: account.signals.near_quota_guard_enabled(),
                    health_score: account.signals.health_score,
                    egress_stability: account.signals.egress_stability,
                    inflight: account.signals.inflight,
                    capacity: account.signals.capacity,
                    has_credential: credential.is_some(),
                    base_url: credential
                        .as_ref()
                        .map(|credential| credential.base_url.clone()),
                    chatgpt_account_id: credential
                        .as_ref()
                        .and_then(|credential| credential.chatgpt_account_id.clone()),
                    egress_group: self.egress_group_label(resolved_route_mode),
                    proxy_enabled: self.route_mode_uses_proxy(resolved_route_mode),
                }
            })
            .collect()
    }

    fn egress_group_label(&self, route_mode: RouteMode) -> String {
        match route_mode {
            RouteMode::Direct => {
                if self.config.direct_proxy_url.is_some() {
                    "direct-proxy".to_string()
                } else {
                    "direct-native".to_string()
                }
            }
            RouteMode::Warp => {
                if self.config.warp_proxy_url.is_some() {
                    "warp-proxy".to_string()
                } else {
                    "warp-native".to_string()
                }
            }
        }
    }

    fn route_mode_uses_proxy(&self, route_mode: RouteMode) -> bool {
        match route_mode {
            RouteMode::Direct => self.config.direct_proxy_url.is_some(),
            RouteMode::Warp => self.config.warp_proxy_url.is_some(),
        }
    }
}

fn gateway_user_view(
    api_key: &GatewayApiKey,
    request_count: usize,
    estimated_spend_usd: f64,
    last_used_at: Option<chrono::DateTime<Utc>>,
) -> GatewayUserView {
    GatewayUserView {
        id: api_key.id,
        tenant_id: api_key.tenant_id,
        name: api_key.name.clone(),
        email: api_key.email.clone(),
        role: api_key.role,
        token_preview: mask_token(&api_key.token),
        default_model: api_key.default_model.clone(),
        reasoning_effort: api_key.reasoning_effort.clone(),
        force_model_override: api_key.force_model_override,
        force_reasoning_effort: api_key.force_reasoning_effort,
        request_count,
        estimated_spend_usd,
        last_used_at,
        created_at: api_key.created_at,
        updated_at: api_key.updated_at,
    }
}

fn build_model_catalog(accounts: &[AccountSummary]) -> Vec<String> {
    let mut models = accounts
        .iter()
        .flat_map(|account| account.models.clone())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    models
}

fn spawn_persistence_writer(
    mut writer_rx: mpsc::Receiver<PersistenceMessage>,
    persistence: Option<Arc<Persistence>>,
) {
    tokio::spawn(async move {
        while let Some(first) = writer_rx.recv().await {
            let mut batch = vec![first];
            while batch.len() < 64 {
                match writer_rx.try_recv() {
                    Ok(message) => batch.push(message),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }

            if let Some(persistence) = persistence.as_ref() {
                if let Err(error) = persistence.persist_batch(&batch).await {
                    warn!(%error, batch_size = batch.len(), "failed to persist writer batch");
                } else {
                    let last_kind = batch.last().map(PersistenceMessage::kind).unwrap_or("-");
                    info!(
                        batch_size = batch.len(),
                        last_kind, "persisted writer batch"
                    );
                }
            } else if !batch.is_empty() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    });
}

fn default_cache_metrics() -> CacheMetrics {
    CacheMetrics {
        cached_tokens: 131_072,
        replay_tokens: 24_576,
        prefix_hit_ratio: 0.81,
        warmup_roi: 2.14,
        static_prefix_tokens: 4_096,
    }
}

fn default_oauth_models() -> Vec<String> {
    vec![
        "gpt-5.4".to_string(),
        "gpt-5.3-codex".to_string(),
        "gpt-5.2".to_string(),
    ]
}

fn default_oauth_base_url() -> String {
    "https://chatgpt.com/backend-api/codex".to_string()
}

fn demo_account(
    tenant_id: &Uuid,
    id: &str,
    label: &str,
    mode: RouteMode,
    quota_headroom: f64,
    health_score: f64,
    egress_stability: f64,
) -> UpstreamAccount {
    UpstreamAccount {
        id: id.to_string(),
        tenant_id: *tenant_id,
        label: label.to_string(),
        models: vec!["gpt-5.4".to_string(), "gpt-5.3-codex".to_string()],
        current_mode: mode,
        signals: SchedulingSignals {
            quota_headroom,
            quota_headroom_5h: quota_headroom,
            quota_headroom_7d: quota_headroom,
            health_score,
            egress_stability,
            fairness_bias: 0.72,
            inflight: 0,
            capacity: 4,
        },
        created_at: Utc::now(),
    }
}

fn build_credential_template(request: &ImportAccountRequest) -> Option<UpstreamCredential> {
    let bearer_token = request
        .bearer_token
        .as_ref()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let now = Utc::now();
    Some(UpstreamCredential {
        account_id: String::new(),
        base_url: request
            .base_url
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        bearer_token,
        chatgpt_account_id: request
            .chatgpt_account_id
            .clone()
            .filter(|value| !value.trim().is_empty()),
        extra_headers: request.extra_headers.clone().unwrap_or_default(),
        created_at: now,
        updated_at: now,
    })
}

fn browser_task_provider_for_credential(credential: Option<&UpstreamCredential>) -> Option<String> {
    let credential = credential?;
    let base_url = credential.base_url.to_ascii_lowercase();
    if base_url.contains("openai.com") || base_url.contains("chatgpt.com") {
        return Some("openai".to_string());
    }
    credential
        .chatgpt_account_id
        .as_ref()
        .map(|_| "openai".to_string())
}

fn browser_task_login_url_for_credential(
    credential: Option<&UpstreamCredential>,
    provider: Option<&str>,
) -> Option<String> {
    if provider == Some("openai") {
        return None;
    }
    credential.map(|credential| credential.base_url.clone())
}

fn mask_token(token: &str) -> String {
    if token.len() <= 10 {
        return "********".to_string();
    }
    format!(
        "{}…{}",
        &token[..6],
        &token[token.len().saturating_sub(4)..]
    )
}

fn mask_endpoint(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let prefix = trimmed
        .split('@')
        .next_back()
        .unwrap_or(trimmed)
        .chars()
        .take(32)
        .collect::<String>();
    if trimmed.chars().count() > 32 {
        format!("{prefix}...")
    } else {
        prefix
    }
}
