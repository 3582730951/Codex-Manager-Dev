use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEFAULT_ISSUER: &str = "https://auth.openai.com";
pub const DEFAULT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const DEFAULT_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub const DEFAULT_ORIGINATOR: &str = "codex_manager_web";

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
    pub id_token: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
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
    let mut url = Url::parse(&format!("{}/oauth/authorize", DEFAULT_ISSUER))
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", DEFAULT_CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", DEFAULT_SCOPE)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", DEFAULT_ORIGINATOR);
    Ok(url.to_string())
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
