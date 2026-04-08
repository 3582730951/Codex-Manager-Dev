use std::{str::FromStr, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use sqlx::{
    PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
    types::Json,
};
use tracing::{info, warn};

use crate::models::{
    AccountRouteState, CacheMetrics, CfIncident, CliLease, ConversationContext, GatewayApiKey,
    RequestLogEntry, RequestLogUsage, RouteMode, SchedulingSignals, Tenant, UpstreamAccount,
    UpstreamCredential,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum PersistenceMessage {
    TenantUpsert(Tenant),
    ApiKeyUpsert(GatewayApiKey),
    AccountUpsert(UpstreamAccount),
    CredentialUpsert(UpstreamCredential),
    RouteStateUpsert(AccountRouteState),
    LeaseUpsert(CliLease),
    LeaseDelete(String),
    IncidentInsert(CfIncident),
    ConversationContextUpsert(ConversationContext),
    CacheMetricsUpsert(CacheMetrics),
    RequestLogInsert(RequestLogEntry),
}

impl PersistenceMessage {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TenantUpsert(_) => "tenant_upsert",
            Self::ApiKeyUpsert(_) => "api_key_upsert",
            Self::AccountUpsert(_) => "account_upsert",
            Self::CredentialUpsert(_) => "credential_upsert",
            Self::RouteStateUpsert(_) => "route_state_upsert",
            Self::LeaseUpsert(_) => "lease_upsert",
            Self::LeaseDelete(_) => "lease_delete",
            Self::IncidentInsert(_) => "incident_insert",
            Self::ConversationContextUpsert(_) => "conversation_context_upsert",
            Self::CacheMetricsUpsert(_) => "cache_metrics_upsert",
            Self::RequestLogInsert(_) => "request_log_insert",
        }
    }
}

#[derive(Debug, Default)]
pub struct PersistenceSnapshot {
    pub tenants: Vec<Tenant>,
    pub api_keys: Vec<GatewayApiKey>,
    pub accounts: Vec<UpstreamAccount>,
    pub credentials: Vec<UpstreamCredential>,
    pub route_states: Vec<AccountRouteState>,
    pub leases: Vec<CliLease>,
    pub cf_incidents: Vec<CfIncident>,
    pub conversation_contexts: Vec<ConversationContext>,
    pub cache_metrics: Option<CacheMetrics>,
    pub request_logs: Vec<RequestLogEntry>,
}

impl PersistenceSnapshot {
    pub fn has_data(&self) -> bool {
        !self.tenants.is_empty()
            || !self.api_keys.is_empty()
            || !self.accounts.is_empty()
            || !self.credentials.is_empty()
            || !self.route_states.is_empty()
            || !self.leases.is_empty()
            || !self.cf_incidents.is_empty()
            || !self.conversation_contexts.is_empty()
            || !self.request_logs.is_empty()
    }
}

#[derive(Clone)]
pub struct Persistence {
    pool: Arc<PgPool>,
}

impl Persistence {
    pub async fn connect(postgres_url: &str) -> Option<Self> {
        let options = match PgConnectOptions::from_str(postgres_url) {
            Ok(options) => options.application_name("codex-manager-server"),
            Err(error) => {
                warn!(%error, "invalid postgres connection string, falling back to memory-only mode");
                return None;
            }
        };
        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(1))
            .connect_with(options)
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                warn!(%error, "postgres unavailable, continuing in memory-only mode");
                return None;
            }
        };
        let persistence = Self {
            pool: Arc::new(pool),
        };
        if let Err(error) = persistence.run_migrations().await {
            warn!(%error, "postgres connected but migrations failed, continuing in memory-only mode");
            return None;
        }
        info!("postgres persistence connected");
        Some(persistence)
    }

    pub async fn load_snapshot(&self) -> Result<PersistenceSnapshot, sqlx::Error> {
        let tenants =
            sqlx::query("select id, slug, name, created_at from tenants order by created_at asc")
                .fetch_all(self.pool.as_ref())
                .await?
                .into_iter()
                .map(|row| -> Result<Tenant, sqlx::Error> {
                    Ok(Tenant {
                        id: row.try_get("id")?,
                        slug: row.try_get("slug")?,
                        name: row.try_get("name")?,
                        created_at: row.try_get("created_at")?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

        let api_keys = sqlx::query(
            "select id, tenant_id, name, email, role, token, default_model, reasoning_effort,
             force_model_override, force_reasoning_effort, created_at, updated_at
             from gateway_api_keys order by created_at asc",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<GatewayApiKey, sqlx::Error> {
            Ok(GatewayApiKey {
                id: row.try_get("id")?,
                tenant_id: row.try_get("tenant_id")?,
                name: row.try_get("name")?,
                email: row
                    .try_get::<Option<String>, _>("email")?
                    .unwrap_or_default(),
                role: crate::models::GatewayUserRole::from_db(
                    row.try_get::<&str, _>("role")?,
                ),
                token: row.try_get("token")?,
                default_model: row.try_get("default_model")?,
                reasoning_effort: row.try_get("reasoning_effort")?,
                force_model_override: row.try_get("force_model_override")?,
                force_reasoning_effort: row.try_get("force_reasoning_effort")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let accounts = sqlx::query(
            "select id, tenant_id, label, models, current_mode, signals, created_at from upstream_accounts order by created_at asc",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<UpstreamAccount, sqlx::Error> {
            let models: Json<Vec<String>> = row.try_get("models")?;
            let signals: Json<SchedulingSignals> = row.try_get("signals")?;
            Ok(UpstreamAccount {
                id: row.try_get("id")?,
                tenant_id: row.try_get("tenant_id")?,
                label: row.try_get("label")?,
                models: models.0,
                current_mode: RouteMode::from_db(row.try_get::<&str, _>("current_mode")?),
                signals: signals.0,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let credentials = sqlx::query(
            "select account_id, base_url, bearer_token, chatgpt_account_id, extra_headers, created_at, updated_at from upstream_credentials order by updated_at asc",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<UpstreamCredential, sqlx::Error> {
            let extra_headers: Json<Vec<(String, String)>> = row.try_get("extra_headers")?;
            Ok(UpstreamCredential {
                account_id: row.try_get("account_id")?,
                base_url: row.try_get("base_url")?,
                bearer_token: row.try_get("bearer_token")?,
                chatgpt_account_id: row.try_get("chatgpt_account_id")?,
                extra_headers: extra_headers.0,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let route_states = sqlx::query(
            "select account_id, route_mode, direct_cf_streak, warp_cf_streak, cooldown_level, cooldown_until, warp_entered_at, last_cf_at, success_streak, last_success_at from account_route_states",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<AccountRouteState, sqlx::Error> {
            let cooldown_level: i32 = row.try_get("cooldown_level")?;
            let direct_cf_streak: i32 = row.try_get("direct_cf_streak")?;
            let warp_cf_streak: i32 = row.try_get("warp_cf_streak")?;
            let success_streak: i32 = row.try_get("success_streak")?;
            Ok(AccountRouteState {
                account_id: row.try_get("account_id")?,
                route_mode: RouteMode::from_db(row.try_get::<&str, _>("route_mode")?),
                direct_cf_streak: direct_cf_streak.max(0) as u32,
                warp_cf_streak: warp_cf_streak.max(0) as u32,
                cooldown_level: cooldown_level.max(0) as usize,
                cooldown_until: row.try_get("cooldown_until")?,
                warp_entered_at: row.try_get("warp_entered_at")?,
                last_cf_at: row.try_get("last_cf_at")?,
                success_streak: success_streak.max(0) as u32,
                last_success_at: row.try_get("last_success_at")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let leases = sqlx::query(
            "select principal_id, tenant_id, account_id, account_label, model, reasoning_effort, route_mode, generation, active_subagents, created_at, last_used_at from cli_leases order by last_used_at desc",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<CliLease, sqlx::Error> {
            let generation: i32 = row.try_get("generation")?;
            let active_subagents: i32 = row.try_get("active_subagents")?;
            Ok(CliLease {
                principal_id: row.try_get("principal_id")?,
                tenant_id: row.try_get("tenant_id")?,
                account_id: row.try_get("account_id")?,
                account_label: row.try_get("account_label")?,
                model: row.try_get("model")?,
                reasoning_effort: row.try_get("reasoning_effort")?,
                route_mode: RouteMode::from_db(row.try_get::<&str, _>("route_mode")?),
                generation: generation.max(0) as u32,
                active_subagents: active_subagents.max(0) as u32,
                created_at: row.try_get("created_at")?,
                last_used_at: row.try_get("last_used_at")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let cf_incidents = sqlx::query(
            "select id, account_id, account_label, route_mode, severity, happened_at, cooldown_level from cf_incidents order by happened_at desc limit 128",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<CfIncident, sqlx::Error> {
            let cooldown_level: i32 = row.try_get("cooldown_level")?;
            Ok(CfIncident {
                id: row.try_get("id")?,
                account_id: row.try_get("account_id")?,
                account_label: row.try_get("account_label")?,
                route_mode: RouteMode::from_db(row.try_get::<&str, _>("route_mode")?),
                severity: row.try_get("severity")?,
                happened_at: row.try_get("happened_at")?,
                cooldown_level: cooldown_level.max(0) as usize,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let conversation_contexts = sqlx::query(
            "select principal_id, model, workflow_spine, turns, updated_at from conversation_contexts order by updated_at desc",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<ConversationContext, sqlx::Error> {
            Ok(ConversationContext {
                principal_id: row.try_get("principal_id")?,
                model: row.try_get("model")?,
                workflow_spine: row.try_get("workflow_spine")?,
                turns: row
                    .try_get::<Json<Vec<crate::models::ContextTurn>>, _>("turns")?
                    .0,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        let cache_metrics = sqlx::query(
            "select cached_tokens, replay_tokens, prefix_hit_ratio, warmup_roi, static_prefix_tokens from cache_metrics where metric_key = 'global'",
        )
        .fetch_optional(self.pool.as_ref())
        .await?
        .map(|row| -> Result<CacheMetrics, sqlx::Error> {
            Ok(CacheMetrics {
                cached_tokens: row.try_get::<i64, _>("cached_tokens")?.max(0) as u64,
                replay_tokens: row.try_get::<i64, _>("replay_tokens")?.max(0) as u64,
                prefix_hit_ratio: row.try_get("prefix_hit_ratio")?,
                warmup_roi: row.try_get("warmup_roi")?,
                static_prefix_tokens: row.try_get::<i64, _>("static_prefix_tokens")?.max(0) as u64,
            })
        })
        .transpose()?;

        let request_logs = sqlx::query(
            "select id, api_key_id, tenant_id, user_name, user_email, principal_id, account_id,
             account_label, method, endpoint, requested_model, effective_model, reasoning_effort,
             route_mode, status_code, usage, estimated_cost_usd, created_at
             from request_logs order by created_at desc limit 512",
        )
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .map(|row| -> Result<RequestLogEntry, sqlx::Error> {
            let status_code: i32 = row.try_get("status_code")?;
            Ok(RequestLogEntry {
                id: row.try_get("id")?,
                api_key_id: row.try_get("api_key_id")?,
                tenant_id: row.try_get("tenant_id")?,
                user_name: row.try_get("user_name")?,
                user_email: row
                    .try_get::<Option<String>, _>("user_email")?
                    .unwrap_or_default(),
                principal_id: row.try_get("principal_id")?,
                account_id: row.try_get("account_id")?,
                account_label: row.try_get("account_label")?,
                method: row.try_get("method")?,
                endpoint: row.try_get("endpoint")?,
                requested_model: row.try_get("requested_model")?,
                effective_model: row.try_get("effective_model")?,
                reasoning_effort: row.try_get("reasoning_effort")?,
                route_mode: RouteMode::from_db(row.try_get::<&str, _>("route_mode")?),
                status_code: status_code.max(0) as u16,
                usage: row
                    .try_get::<Json<RequestLogUsage>, _>("usage")
                    .map(|value| value.0)
                    .unwrap_or_default(),
                estimated_cost_usd: row.try_get("estimated_cost_usd")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

        Ok(PersistenceSnapshot {
            tenants,
            api_keys,
            accounts,
            credentials,
            route_states,
            leases,
            cf_incidents,
            conversation_contexts,
            cache_metrics,
            request_logs,
        })
    }

    pub async fn persist_batch(&self, batch: &[PersistenceMessage]) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        for message in batch {
            match message {
                PersistenceMessage::TenantUpsert(tenant) => {
                    sqlx::query(
                        "insert into tenants (id, slug, name, created_at) values ($1, $2, $3, $4)
                         on conflict (id) do update set slug = excluded.slug, name = excluded.name, created_at = excluded.created_at",
                    )
                    .bind(tenant.id)
                    .bind(&tenant.slug)
                    .bind(&tenant.name)
                    .bind(tenant.created_at)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::ApiKeyUpsert(api_key) => {
                    sqlx::query(
                        "insert into gateway_api_keys (
                            id, tenant_id, name, email, role, token, default_model, reasoning_effort,
                            force_model_override, force_reasoning_effort, created_at, updated_at
                         ) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                         on conflict (id) do update set
                         tenant_id = excluded.tenant_id,
                         name = excluded.name,
                         email = excluded.email,
                         role = excluded.role,
                         token = excluded.token,
                         default_model = excluded.default_model,
                         reasoning_effort = excluded.reasoning_effort,
                         force_model_override = excluded.force_model_override,
                         force_reasoning_effort = excluded.force_reasoning_effort,
                         created_at = excluded.created_at,
                         updated_at = excluded.updated_at",
                    )
                    .bind(api_key.id)
                    .bind(api_key.tenant_id)
                    .bind(&api_key.name)
                    .bind(&api_key.email)
                    .bind(api_key.role.as_str())
                    .bind(&api_key.token)
                    .bind(&api_key.default_model)
                    .bind(&api_key.reasoning_effort)
                    .bind(api_key.force_model_override)
                    .bind(api_key.force_reasoning_effort)
                    .bind(api_key.created_at)
                    .bind(api_key.updated_at)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::AccountUpsert(account) => {
                    sqlx::query(
                        "insert into upstream_accounts (id, tenant_id, label, models, current_mode, signals, created_at) values ($1, $2, $3, $4, $5, $6, $7)
                         on conflict (id) do update set tenant_id = excluded.tenant_id, label = excluded.label, models = excluded.models, current_mode = excluded.current_mode, signals = excluded.signals, created_at = excluded.created_at",
                    )
                    .bind(&account.id)
                    .bind(account.tenant_id)
                    .bind(&account.label)
                    .bind(Json(account.models.clone()))
                    .bind(account.current_mode.as_str())
                    .bind(Json(account.signals.clone()))
                    .bind(account.created_at)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::CredentialUpsert(credential) => {
                    sqlx::query(
                        "insert into upstream_credentials (account_id, base_url, bearer_token, chatgpt_account_id, extra_headers, created_at, updated_at)
                         values ($1, $2, $3, $4, $5, $6, $7)
                         on conflict (account_id) do update set base_url = excluded.base_url, bearer_token = excluded.bearer_token,
                         chatgpt_account_id = excluded.chatgpt_account_id, extra_headers = excluded.extra_headers, created_at = excluded.created_at, updated_at = excluded.updated_at",
                    )
                    .bind(&credential.account_id)
                    .bind(&credential.base_url)
                    .bind(&credential.bearer_token)
                    .bind(&credential.chatgpt_account_id)
                    .bind(Json(credential.extra_headers.clone()))
                    .bind(credential.created_at)
                    .bind(credential.updated_at)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::RouteStateUpsert(route_state) => {
                    sqlx::query(
                        "insert into account_route_states (account_id, route_mode, direct_cf_streak, warp_cf_streak, cooldown_level, cooldown_until, warp_entered_at, last_cf_at, success_streak, last_success_at)
                         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                         on conflict (account_id) do update set route_mode = excluded.route_mode, direct_cf_streak = excluded.direct_cf_streak, warp_cf_streak = excluded.warp_cf_streak,
                         cooldown_level = excluded.cooldown_level, cooldown_until = excluded.cooldown_until, warp_entered_at = excluded.warp_entered_at, last_cf_at = excluded.last_cf_at,
                         success_streak = excluded.success_streak, last_success_at = excluded.last_success_at",
                    )
                    .bind(&route_state.account_id)
                    .bind(route_state.route_mode.as_str())
                    .bind(route_state.direct_cf_streak as i32)
                    .bind(route_state.warp_cf_streak as i32)
                    .bind(route_state.cooldown_level as i32)
                    .bind(route_state.cooldown_until)
                    .bind(route_state.warp_entered_at)
                    .bind(route_state.last_cf_at)
                    .bind(route_state.success_streak as i32)
                    .bind(route_state.last_success_at)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::LeaseUpsert(lease) => {
                    sqlx::query(
                        "insert into cli_leases (principal_id, tenant_id, account_id, account_label, model, reasoning_effort, route_mode, generation, active_subagents, created_at, last_used_at)
                         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                         on conflict (principal_id) do update set tenant_id = excluded.tenant_id, account_id = excluded.account_id, account_label = excluded.account_label,
                         model = excluded.model, reasoning_effort = excluded.reasoning_effort, route_mode = excluded.route_mode, generation = excluded.generation,
                         active_subagents = excluded.active_subagents, created_at = excluded.created_at, last_used_at = excluded.last_used_at",
                    )
                    .bind(&lease.principal_id)
                    .bind(lease.tenant_id)
                    .bind(&lease.account_id)
                    .bind(&lease.account_label)
                    .bind(&lease.model)
                    .bind(&lease.reasoning_effort)
                    .bind(lease.route_mode.as_str())
                    .bind(lease.generation as i32)
                    .bind(lease.active_subagents as i32)
                    .bind(lease.created_at)
                    .bind(lease.last_used_at)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::LeaseDelete(principal_id) => {
                    sqlx::query("delete from cli_leases where principal_id = $1")
                        .bind(principal_id)
                        .execute(&mut *tx)
                        .await?;
                }
                PersistenceMessage::IncidentInsert(incident) => {
                    sqlx::query(
                        "insert into cf_incidents (id, account_id, account_label, route_mode, severity, happened_at, cooldown_level)
                         values ($1, $2, $3, $4, $5, $6, $7)
                         on conflict (id) do nothing",
                    )
                    .bind(&incident.id)
                    .bind(&incident.account_id)
                    .bind(&incident.account_label)
                    .bind(incident.route_mode.as_str())
                    .bind(&incident.severity)
                    .bind(incident.happened_at)
                    .bind(incident.cooldown_level as i32)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::ConversationContextUpsert(context) => {
                    sqlx::query(
                        "insert into conversation_contexts (principal_id, model, workflow_spine, turns, updated_at)
                         values ($1, $2, $3, $4, $5)
                         on conflict (principal_id) do update set model = excluded.model, workflow_spine = excluded.workflow_spine,
                         turns = excluded.turns, updated_at = excluded.updated_at",
                    )
                    .bind(&context.principal_id)
                    .bind(&context.model)
                    .bind(&context.workflow_spine)
                    .bind(Json(context.turns.clone()))
                    .bind(context.updated_at)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::CacheMetricsUpsert(metrics) => {
                    sqlx::query(
                        "insert into cache_metrics (metric_key, cached_tokens, replay_tokens, prefix_hit_ratio, warmup_roi, static_prefix_tokens, updated_at)
                         values ('global', $1, $2, $3, $4, $5, now())
                         on conflict (metric_key) do update set cached_tokens = excluded.cached_tokens, replay_tokens = excluded.replay_tokens,
                         prefix_hit_ratio = excluded.prefix_hit_ratio, warmup_roi = excluded.warmup_roi, static_prefix_tokens = excluded.static_prefix_tokens, updated_at = now()",
                    )
                    .bind(metrics.cached_tokens as i64)
                    .bind(metrics.replay_tokens as i64)
                    .bind(metrics.prefix_hit_ratio)
                    .bind(metrics.warmup_roi)
                    .bind(metrics.static_prefix_tokens as i64)
                    .execute(&mut *tx)
                    .await?;
                }
                PersistenceMessage::RequestLogInsert(log) => {
                    sqlx::query(
                        "insert into request_logs (
                            id, api_key_id, tenant_id, user_name, user_email, principal_id, account_id,
                            account_label, method, endpoint, requested_model, effective_model,
                            reasoning_effort, route_mode, status_code, usage, estimated_cost_usd, created_at
                         ) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
                         on conflict (id) do nothing",
                    )
                    .bind(&log.id)
                    .bind(log.api_key_id)
                    .bind(log.tenant_id)
                    .bind(&log.user_name)
                    .bind(&log.user_email)
                    .bind(&log.principal_id)
                    .bind(&log.account_id)
                    .bind(&log.account_label)
                    .bind(&log.method)
                    .bind(&log.endpoint)
                    .bind(&log.requested_model)
                    .bind(&log.effective_model)
                    .bind(&log.reasoning_effort)
                    .bind(log.route_mode.as_str())
                    .bind(log.status_code as i32)
                    .bind(Json(log.usage.clone()))
                    .bind(log.estimated_cost_usd)
                    .bind(log.created_at)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
        tx.commit().await?;
        Ok(())
    }

    async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        for migration in [
            include_str!("../migrations/0001_init.sql"),
            include_str!("../migrations/0002_upstream_credentials.sql"),
            include_str!("../migrations/0003_conversation_contexts.sql"),
            include_str!("../migrations/0004_gateway_users_and_request_logs.sql"),
        ] {
            for statement in migration
                .split("\n-- statement-break\n")
                .map(str::trim)
                .filter(|statement| !statement.is_empty())
            {
                sqlx::query(statement).execute(self.pool.as_ref()).await?;
            }
        }
        Ok(())
    }
}
