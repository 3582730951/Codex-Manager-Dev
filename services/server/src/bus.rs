use std::sync::Arc;

use futures_util::StreamExt;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{
    config::Config,
    models::{
        AccountRouteState, CacheMetrics, CfIncident, CliLease, ConversationContext,
        ConversationThread, GatewayApiKey, RequestLogEntry, Tenant, ThreadEdge, UpstreamAccount,
        UpstreamCredential,
    },
    state::RuntimeState,
    storage::PersistenceMessage,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BusEnvelope {
    origin_id: String,
    message: PersistenceMessage,
}

pub async fn connect(
    config: &Config,
    runtime: Arc<RuntimeState>,
) -> Option<mpsc::Sender<PersistenceMessage>> {
    let client = match redis::Client::open(config.redis_url.clone()) {
        Ok(client) => client,
        Err(error) => {
            warn!(%error, "invalid redis url, distributed bus disabled");
            return None;
        }
    };

    let publish_connection = match client.get_multiplexed_async_connection().await {
        Ok(connection) => connection,
        Err(error) => {
            warn!(%error, "redis publish connection unavailable, distributed bus disabled");
            return None;
        }
    };

    let mut pubsub = match client.get_async_pubsub().await {
        Ok(pubsub) => pubsub,
        Err(error) => {
            warn!(%error, "redis pubsub connection unavailable, distributed bus disabled");
            return None;
        }
    };

    if let Err(error) = pubsub.subscribe(config.redis_channel.as_str()).await {
        warn!(%error, channel = %config.redis_channel, "failed to subscribe redis control channel");
        return None;
    }

    let (bus_tx, mut bus_rx) = mpsc::channel::<PersistenceMessage>(1024);
    let origin_id = config.instance_id.clone();
    let channel = config.redis_channel.clone();
    let publish_origin = origin_id.clone();
    let mut publisher = publish_connection;

    tokio::spawn(async move {
        while let Some(message) = bus_rx.recv().await {
            let envelope = BusEnvelope {
                origin_id: publish_origin.clone(),
                message,
            };
            let payload = match serde_json::to_string(&envelope) {
                Ok(payload) => payload,
                Err(error) => {
                    warn!(%error, "failed to serialize bus envelope");
                    continue;
                }
            };
            if let Err(error) = publisher.publish::<_, _, i64>(&channel, payload).await {
                warn!(%error, channel = %channel, "failed to publish redis control event");
            }
        }
    });

    tokio::spawn(async move {
        let mut stream = pubsub.on_message();
        while let Some(message) = stream.next().await {
            let payload = match message.get_payload::<String>() {
                Ok(payload) => payload,
                Err(error) => {
                    warn!(%error, "failed to decode redis control event");
                    continue;
                }
            };
            let envelope = match serde_json::from_str::<BusEnvelope>(&payload) {
                Ok(envelope) => envelope,
                Err(error) => {
                    warn!(%error, "failed to deserialize redis control event");
                    continue;
                }
            };
            if envelope.origin_id == origin_id {
                continue;
            }
            apply_message(&runtime, envelope.message).await;
        }
        warn!("redis pubsub stream ended");
    });

    info!(channel = %config.redis_channel, instance_id = %config.instance_id, "redis distributed bus connected");
    Some(bus_tx)
}

async fn apply_message(runtime: &Arc<RuntimeState>, message: PersistenceMessage) {
    match message {
        PersistenceMessage::TenantUpsert(tenant) => upsert_tenant(runtime, tenant).await,
        PersistenceMessage::ApiKeyUpsert(api_key) => upsert_api_key(runtime, api_key).await,
        PersistenceMessage::AccountUpsert(account) => upsert_account(runtime, account).await,
        PersistenceMessage::CredentialUpsert(credential) => {
            upsert_credential(runtime, credential).await
        }
        PersistenceMessage::RouteStateUpsert(route_state) => {
            upsert_route_state(runtime, route_state).await
        }
        PersistenceMessage::LeaseUpsert(lease) => upsert_lease(runtime, lease).await,
        PersistenceMessage::LeaseDelete(principal_id) => delete_lease(runtime, &principal_id).await,
        PersistenceMessage::IncidentInsert(incident) => insert_incident(runtime, incident).await,
        PersistenceMessage::ConversationContextUpsert(context) => {
            upsert_conversation_context(runtime, context).await
        }
        PersistenceMessage::ConversationThreadUpsert(thread) => {
            upsert_conversation_thread(runtime, thread).await
        }
        PersistenceMessage::ThreadEdgeUpsert(edge) => upsert_thread_edge(runtime, edge).await,
        PersistenceMessage::CacheMetricsUpsert(metrics) => {
            replace_cache_metrics(runtime, metrics).await
        }
        PersistenceMessage::RequestLogInsert(log) => insert_request_log(runtime, log).await,
    }
}

async fn upsert_tenant(runtime: &Arc<RuntimeState>, tenant: Tenant) {
    runtime.tenants.write().await.insert(tenant.id, tenant);
}

async fn upsert_api_key(runtime: &Arc<RuntimeState>, api_key: GatewayApiKey) {
    runtime
        .api_keys
        .write()
        .await
        .insert(api_key.token.clone(), api_key);
}

async fn upsert_account(runtime: &Arc<RuntimeState>, account: UpstreamAccount) {
    runtime
        .accounts
        .write()
        .await
        .insert(account.id.clone(), account);
}

async fn upsert_credential(runtime: &Arc<RuntimeState>, credential: UpstreamCredential) {
    runtime
        .credentials
        .write()
        .await
        .insert(credential.account_id.clone(), credential);
}

async fn upsert_route_state(runtime: &Arc<RuntimeState>, route_state: AccountRouteState) {
    {
        runtime
            .route_states
            .write()
            .await
            .insert(route_state.account_id.clone(), route_state.clone());
    }
    if let Some(account) = runtime
        .accounts
        .write()
        .await
        .get_mut(&route_state.account_id)
    {
        account.current_mode = route_state.route_mode;
    }
}

async fn upsert_lease(runtime: &Arc<RuntimeState>, lease: CliLease) {
    runtime
        .leases
        .write()
        .await
        .insert(lease.principal_id.clone(), lease);
}

async fn delete_lease(runtime: &Arc<RuntimeState>, principal_id: &str) {
    runtime.leases.write().await.remove(principal_id);
}

async fn insert_incident(runtime: &Arc<RuntimeState>, incident: CfIncident) {
    let mut incidents = runtime.cf_incidents.write().await;
    if incidents.iter().any(|existing| existing.id == incident.id) {
        return;
    }
    incidents.insert(0, incident);
    incidents.truncate(128);
}

async fn replace_cache_metrics(runtime: &Arc<RuntimeState>, metrics: CacheMetrics) {
    *runtime.cache_metrics.write().await = metrics;
}

async fn upsert_conversation_context(runtime: &Arc<RuntimeState>, context: ConversationContext) {
    runtime
        .conversation_contexts
        .write()
        .await
        .insert(context.principal_id.clone(), context);
}

async fn upsert_conversation_thread(runtime: &Arc<RuntimeState>, thread: ConversationThread) {
    runtime
        .conversation_threads
        .write()
        .await
        .insert(thread.thread_id.clone(), thread);
}

async fn upsert_thread_edge(runtime: &Arc<RuntimeState>, edge: ThreadEdge) {
    let mut edges = runtime.thread_edges.write().await;
    if edges.iter().any(|existing| {
        existing.parent_thread_id == edge.parent_thread_id
            && existing.child_thread_id == edge.child_thread_id
            && existing.relation == edge.relation
    }) {
        return;
    }
    edges.push(edge);
}

async fn insert_request_log(runtime: &Arc<RuntimeState>, log: RequestLogEntry) {
    let mut logs = runtime.request_logs.write().await;
    if logs.iter().any(|existing| existing.id == log.id) {
        return;
    }
    logs.insert(0, log);
    logs.truncate(512);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::models::{CliLease, RouteMode};

    #[tokio::test]
    async fn lease_upsert_updates_runtime_state() {
        let runtime = Arc::new(RuntimeState::default());
        let lease = CliLease {
            principal_id: "tenant:demo/principal:test".to_string(),
            tenant_id: Uuid::new_v4(),
            account_id: "acc_demo_9".to_string(),
            account_label: "Aurora".to_string(),
            model: "gpt-5.4".to_string(),
            reasoning_effort: Some("high".to_string()),
            route_mode: RouteMode::Direct,
            generation: 2,
            active_subagents: 1,
            created_at: Utc::now(),
            last_used_at: Utc::now(),
        };

        apply_message(&runtime, PersistenceMessage::LeaseUpsert(lease.clone())).await;

        let stored = runtime
            .leases
            .read()
            .await
            .get(&lease.principal_id)
            .cloned();
        assert!(stored.is_some());
        assert_eq!(stored.unwrap().account_id, "acc_demo_9");
    }

    #[tokio::test]
    async fn incident_insert_is_idempotent() {
        let runtime = Arc::new(RuntimeState::default());
        let incident = CfIncident {
            id: "incident-1".to_string(),
            account_id: "acc_demo_1".to_string(),
            account_label: "Meridian".to_string(),
            route_mode: RouteMode::Warp,
            severity: "cooldown".to_string(),
            happened_at: Utc::now(),
            cooldown_level: 2,
        };

        apply_message(
            &runtime,
            PersistenceMessage::IncidentInsert(incident.clone()),
        )
        .await;
        apply_message(&runtime, PersistenceMessage::IncidentInsert(incident)).await;

        assert_eq!(runtime.cf_incidents.read().await.len(), 1);
    }

    #[tokio::test]
    async fn lease_delete_removes_runtime_state() {
        let runtime = Arc::new(RuntimeState::default());
        let lease = CliLease {
            principal_id: "tenant:demo/principal:test".to_string(),
            tenant_id: Uuid::new_v4(),
            account_id: "acc_demo_9".to_string(),
            account_label: "Aurora".to_string(),
            model: "gpt-5.4".to_string(),
            reasoning_effort: Some("high".to_string()),
            route_mode: RouteMode::Direct,
            generation: 2,
            active_subagents: 1,
            created_at: Utc::now(),
            last_used_at: Utc::now(),
        };

        apply_message(&runtime, PersistenceMessage::LeaseUpsert(lease.clone())).await;
        apply_message(
            &runtime,
            PersistenceMessage::LeaseDelete(lease.principal_id.clone()),
        )
        .await;

        assert!(
            runtime
                .leases
                .read()
                .await
                .get(&lease.principal_id)
                .is_none()
        );
    }
}
