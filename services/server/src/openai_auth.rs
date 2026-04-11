use std::collections::BTreeMap;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{
    ManagedCreditsSnapshot, ManagedRateLimitSnapshot, ManagedRateLimitWindow,
    ManagedSpendControlSnapshot,
};

pub const DEFAULT_ISSUER: &str = "https://auth.openai.com";
pub const DEFAULT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const DEFAULT_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";

#[derive(Debug, Clone)]
pub struct PkceCodes {
    pub code_verifier: String,
    pub code_challenge: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdTokenClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    pub auth: Option<AuthClaims>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthClaims {
    #[serde(default)]
    pub chatgpt_account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenResponse {
    #[serde(default)]
    pub id_token: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ManagedChatgptSnapshot {
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub workspace_role: Option<String>,
    pub is_workspace_owner: Option<bool>,
    pub rate_limits: Option<ManagedRateLimitSnapshot>,
    pub rate_limits_by_limit_id: BTreeMap<String, ManagedRateLimitSnapshot>,
    pub chatgpt_account_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ManagedChatgptSnapshotError {
    pub message: String,
    pub unauthorized: bool,
}

pub fn generate_pkce() -> PkceCodes {
    let mut bytes = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(digest);
    PkceCodes {
        code_verifier,
        code_challenge,
    }
}

pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn build_authorize_url(
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
) -> Result<String, String> {
    let query = [
        ("response_type", "code".to_string()),
        ("client_id", DEFAULT_CLIENT_ID.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("scope", DEFAULT_SCOPE.to_string()),
        ("code_challenge", code_challenge.to_string()),
        ("code_challenge_method", "S256".to_string()),
        ("id_token_add_organizations", "true".to_string()),
        ("codex_cli_simplified_flow", "true".to_string()),
        ("state", state.to_string()),
        ("originator", DEFAULT_ORIGINATOR.to_string()),
    ]
    .into_iter()
    .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
    .collect::<Vec<_>>()
    .join("&");

    Ok(format!("{}/oauth/authorize?{}", DEFAULT_ISSUER, query))
}

pub fn parse_id_token_claims(token: &str) -> Result<IdTokenClaims, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "invalid id_token payload".to_string())?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice::<IdTokenClaims>(&decoded).map_err(|error| error.to_string())
}

pub fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value = serde_json::from_slice::<serde_json::Value>(&decoded).ok()?;
    if let Some(id) = value
        .get("chatgpt_account_id")
        .and_then(|value| value.as_str())
    {
        if !id.trim().is_empty() {
            return Some(id.to_string());
        }
    }
    value
        .get("https://api.openai.com/auth")
        .and_then(|value| value.get("chatgpt_account_id"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
}

pub async fn exchange_code_for_tokens(
    redirect_uri: &str,
    code_verifier: &str,
    code: &str,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/oauth/token", DEFAULT_ISSUER))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", DEFAULT_CLIENT_ID),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI token 接口返回 {status}: {body}"));
    }

    response
        .json::<TokenResponse>()
        .await
        .map_err(|error| error.to_string())
}

pub async fn refresh_access_token(refresh_token: &str) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/oauth/token", DEFAULT_ISSUER))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", DEFAULT_CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI refresh 接口返回 {status}: {body}"));
    }

    response
        .json::<TokenResponse>()
        .await
        .map_err(|error| error.to_string())
}

pub fn encrypt_secret(secret: &str, plaintext: &str) -> Result<String, String> {
    let key = decode_secret_key(secret)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| error.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut packed = nonce_bytes.to_vec();
    packed.extend(ciphertext);
    Ok(URL_SAFE_NO_PAD.encode(packed))
}

pub fn decrypt_secret(secret: &str, ciphertext: &str) -> Result<String, String> {
    let key = decode_secret_key(secret)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| error.to_string())?;
    let packed = URL_SAFE_NO_PAD
        .decode(ciphertext)
        .map_err(|error| error.to_string())?;
    if packed.len() < 13 {
        return Err("encrypted secret payload invalid".to_string());
    }
    let (nonce_bytes, payload) = packed.split_at(12);
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), payload)
        .map_err(|error| error.to_string())?;
    String::from_utf8(plaintext).map_err(|error| error.to_string())
}

pub fn extract_email_from_token(token: &str) -> Option<String> {
    let value = parse_jwt_payload(token).ok()?;
    value
        .get("email")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

pub fn extract_plan_type_from_token(token: &str) -> Option<String> {
    let value = parse_jwt_payload(token).ok()?;
    extract_string_recursive(
        &value,
        &[
            "plan_type",
            "planType",
            "subscription_tier",
            "subscriptionTier",
            "tier",
        ],
    )
}

pub async fn fetch_managed_chatgpt_snapshot(
    base_url: &str,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<ManagedChatgptSnapshot, ManagedChatgptSnapshotError> {
    let Some(api_base) = chatgpt_backend_base(base_url) else {
        return Err(ManagedChatgptSnapshotError {
            message: "当前账号不是 ChatGPT 受管后端。".to_string(),
            unauthorized: false,
        });
    };
    let client = reqwest::Client::new();
    let usage_value = fetch_json_with_auth(
        &client,
        &format!("{api_base}/wham/usage"),
        access_token,
        chatgpt_account_id,
    )
    .await?;
    let workspace_value = fetch_json_with_auth(
        &client,
        &format!("{api_base}/accounts/check/v4"),
        access_token,
        chatgpt_account_id,
    )
    .await
    .ok();

    let plan_type = extract_plan_type_from_usage_value(&usage_value)
        .or_else(|| extract_plan_type_from_token(access_token));
    let workspace_role = workspace_value
        .as_ref()
        .and_then(|value| extract_workspace_role(value, chatgpt_account_id));
    let is_workspace_owner = workspace_role
        .as_deref()
        .map(|role| matches!(role, "account-owner" | "account-admin"));
    let (rate_limits, rate_limits_by_limit_id) = extract_rate_limits_from_usage(&usage_value);

    Ok(ManagedChatgptSnapshot {
        email: extract_email_from_token(access_token),
        plan_type,
        workspace_role,
        is_workspace_owner,
        rate_limits,
        rate_limits_by_limit_id,
        chatgpt_account_id: chatgpt_account_id.map(str::to_string),
    })
}

fn decode_secret_key(secret: &str) -> Result<[u8; 32], String> {
    if secret.len() == 32 {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(secret.as_bytes());
        return Ok(bytes);
    }

    let decoded = URL_SAFE_NO_PAD
        .decode(secret)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(secret))
        .map_err(|_| "CMGR_ACCOUNT_ENCRYPTION_KEY 必须是 32 字节明文或 base64 编码的 32 字节密钥".to_string())?;
    let key = decoded
        .get(..32)
        .ok_or_else(|| "CMGR_ACCOUNT_ENCRYPTION_KEY 长度不足 32 字节".to_string())?;
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(key);
    Ok(bytes)
}

fn parse_jwt_payload(token: &str) -> Result<serde_json::Value, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "invalid JWT payload".to_string())?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice::<serde_json::Value>(&decoded).map_err(|error| error.to_string())
}

async fn fetch_json_with_auth(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<serde_json::Value, ManagedChatgptSnapshotError> {
    let mut request = client.get(url).bearer_auth(access_token);
    if let Some(chatgpt_account_id) = chatgpt_account_id {
        request = request.header("ChatGPT-Account-Id", chatgpt_account_id);
    }
    let response = request
        .send()
        .await
        .map_err(|error| ManagedChatgptSnapshotError {
            message: error.to_string(),
            unauthorized: false,
        })?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ManagedChatgptSnapshotError {
            message: format!("{} {}", status, body),
            unauthorized: matches!(
                status,
                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
            ),
        });
    }
    serde_json::from_str(&body).map_err(|error| ManagedChatgptSnapshotError {
        message: error.to_string(),
        unauthorized: false,
    })
}

fn chatgpt_backend_base(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim_end_matches('/');
    if let Some((prefix, _)) = trimmed.split_once("/backend-api") {
        return Some(format!("{prefix}/backend-api"));
    }
    if trimmed.contains("chatgpt.com") || trimmed.contains("chat.openai.com") {
        return Some(format!("{trimmed}/backend-api"));
    }
    None
}

fn extract_rate_limits_from_usage(
    value: &serde_json::Value,
) -> (
    Option<ManagedRateLimitSnapshot>,
    BTreeMap<String, ManagedRateLimitSnapshot>,
) {
    let plan_type = extract_plan_type_from_usage_value(value);
    let primary = ManagedRateLimitSnapshot {
        limit_id: Some("codex".to_string()),
        limit_name: None,
        primary: value
            .pointer("/rate_limit/primary_window")
            .and_then(parse_rate_limit_window),
        secondary: value
            .pointer("/rate_limit/secondary_window")
            .and_then(parse_rate_limit_window),
        credits: value.get("credits").and_then(parse_credits_snapshot),
        spend_control: value.get("spend_control").and_then(parse_spend_control_snapshot),
        plan_type: plan_type.clone(),
    };

    let mut by_limit_id = BTreeMap::new();
    by_limit_id.insert("codex".to_string(), primary.clone());
    if let Some(items) = value.get("additional_rate_limits").and_then(serde_json::Value::as_array) {
        for item in items {
            let limit_id = item
                .get("metered_feature")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let rate_limit = item.get("rate_limit").unwrap_or(item);
            by_limit_id.insert(
                limit_id.clone(),
                ManagedRateLimitSnapshot {
                    limit_id: Some(limit_id),
                    limit_name: item
                        .get("limit_name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    primary: rate_limit
                        .get("primary_window")
                        .and_then(parse_rate_limit_window),
                    secondary: rate_limit
                        .get("secondary_window")
                        .and_then(parse_rate_limit_window),
                    credits: None,
                    spend_control: None,
                    plan_type: plan_type.clone(),
                },
            );
        }
    }

    (Some(primary), by_limit_id)
}

fn parse_rate_limit_window(value: &serde_json::Value) -> Option<ManagedRateLimitWindow> {
    Some(ManagedRateLimitWindow {
        used_percent: value
            .get("used_percent")
            .and_then(serde_json::Value::as_f64)
            .map(|value| value.round() as i32)
            .or_else(|| {
                value
                    .get("usedPercent")
                    .and_then(serde_json::Value::as_i64)
                    .map(|value| value as i32)
            })?,
        window_duration_mins: value
            .get("limit_window_seconds")
            .and_then(serde_json::Value::as_i64)
            .map(|value| (value + 59) / 60)
            .or_else(|| {
                value
                    .get("windowDurationMins")
                    .and_then(serde_json::Value::as_i64)
            }),
        resets_at: value
            .get("reset_at")
            .and_then(serde_json::Value::as_i64)
            .or_else(|| value.get("resetsAt").and_then(serde_json::Value::as_i64)),
    })
}

fn parse_credits_snapshot(value: &serde_json::Value) -> Option<ManagedCreditsSnapshot> {
    Some(ManagedCreditsSnapshot {
        has_credits: value
            .get("has_credits")
            .or_else(|| value.get("hasCredits"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        unlimited: value
            .get("unlimited")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        balance: value
            .get("balance")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

fn parse_spend_control_snapshot(
    value: &serde_json::Value,
) -> Option<ManagedSpendControlSnapshot> {
    Some(ManagedSpendControlSnapshot {
        reached: value
            .get("reached")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

fn extract_plan_type_from_usage_value(value: &serde_json::Value) -> Option<String> {
    extract_string_recursive(
        value,
        &[
            "plan_type",
            "planType",
            "subscription_tier",
            "subscriptionTier",
            "tier",
            "type",
        ],
    )
}

fn extract_workspace_role(
    value: &serde_json::Value,
    current_account_id: Option<&str>,
) -> Option<String> {
    let accounts = value.get("accounts")?.as_object()?;
    if let Some(account_id) = current_account_id {
        return accounts
            .get(account_id)
            .and_then(extract_workspace_role_from_account_entry);
    }
    value
        .get("account_ordering")
        .and_then(serde_json::Value::as_array)
        .and_then(|ordering| {
            ordering.iter().find_map(|account_id| {
                accounts
                    .get(account_id.as_str()?)
                    .and_then(extract_workspace_role_from_account_entry)
            })
        })
        .or_else(|| {
            if accounts.len() == 1 {
                accounts
                    .values()
                    .next()
                    .and_then(extract_workspace_role_from_account_entry)
            } else {
                None
            }
        })
}

fn extract_workspace_role_from_account_entry(value: &serde_json::Value) -> Option<String> {
    value
        .get("account")
        .and_then(|account| {
            account
                .get("account_user_role")
                .or_else(|| account.get("accountUserRole"))
        })
        .and_then(serde_json::Value::as_str)
        .map(|role| match role {
            "account_owner" => "account-owner".to_string(),
            "account_admin" => "account-admin".to_string(),
            "member" => "standard-user".to_string(),
            other => other.replace('_', "-"),
        })
}

fn extract_string_recursive(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    if let Some(found) = value.as_object().and_then(|object| {
        keys.iter().find_map(|key| {
            object
                .get(*key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
    }) {
        return Some(found);
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|item| extract_string_recursive(item, keys)),
        serde_json::Value::Object(object) => object
            .values()
            .find_map(|item| extract_string_recursive(item, keys)),
        _ => None,
    }
}
