use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: IpAddr,
    pub data_port: u16,
    pub admin_port: u16,
    pub max_data_plane_body_bytes: usize,
    pub postgres_url: String,
    pub redis_url: String,
    pub redis_channel: String,
    pub instance_id: String,
    pub browser_assist_url: String,
    pub heartbeat_seconds: u64,
    pub enable_demo_seed: bool,
    pub account_encryption_key: Option<String>,
    pub direct_proxy_url: Option<String>,
    pub warp_proxy_url: Option<String>,
    pub browser_assist_direct_proxy_url: Option<String>,
    pub browser_assist_warp_proxy_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            bind_addr: std::env::var("CMGR_SERVER_BIND_ADDR")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            data_port: std::env::var("CMGR_SERVER_DATA_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(8080),
            admin_port: std::env::var("CMGR_SERVER_ADMIN_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(8081),
            max_data_plane_body_bytes: std::env::var("CMGR_SERVER_MAX_DATA_BODY_BYTES")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(64 * 1024 * 1024),
            postgres_url: std::env::var("CMGR_POSTGRES_URL").unwrap_or_else(|_| {
                "postgres://codex_manager:codex_manager@localhost:5432/codex_manager".to_string()
            }),
            redis_url: std::env::var("CMGR_REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
            redis_channel: std::env::var("CMGR_REDIS_CHANNEL")
                .unwrap_or_else(|_| "cmgr:control-events".to_string()),
            instance_id: std::env::var("CMGR_INSTANCE_ID")
                .unwrap_or_else(|_| format!("cmgr-{}", Uuid::new_v4().simple())),
            browser_assist_url: std::env::var("CMGR_BROWSER_ASSIST_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8090".to_string()),
            heartbeat_seconds: std::env::var("CMGR_GATEWAY_HEARTBEAT_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(5),
            enable_demo_seed: std::env::var("CMGR_ENABLE_DEMO_SEED")
                .ok()
                .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false),
            account_encryption_key: std::env::var("CMGR_ACCOUNT_ENCRYPTION_KEY")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            direct_proxy_url: read_proxy_env("CMGR_DIRECT_PROXY_URL"),
            warp_proxy_url: read_proxy_env("CMGR_WARP_PROXY_URL"),
            browser_assist_direct_proxy_url: read_proxy_env("CMGR_BROWSER_ASSIST_DIRECT_PROXY_URL"),
            browser_assist_warp_proxy_url: read_proxy_env("CMGR_BROWSER_ASSIST_WARP_PROXY_URL"),
        }
    }

    pub fn data_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_addr, self.data_port)
    }

    pub fn admin_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_addr, self.admin_port)
    }
}

fn read_proxy_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
