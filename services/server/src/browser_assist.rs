use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::models::{BrowserTask, RouteMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTaskPayload {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserTaskEnvelope {
    task: BrowserTaskWire,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserTaskList {
    items: Vec<BrowserTaskWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserTaskWire {
    id: String,
    kind: String,
    account_id: Option<String>,
    account_label: Option<String>,
    provider: Option<String>,
    route_mode: Option<RouteMode>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    notes: Option<String>,
    profile_dir: Option<String>,
    screenshot_path: Option<String>,
    storage_state_path: Option<String>,
    final_url: Option<String>,
    last_error: Option<String>,
    steps: Vec<String>,
}

pub async fn list_tasks(browser_assist_url: &str) -> Vec<BrowserTask> {
    let url = format!("{}/v1/tasks", browser_assist_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let response = match client.get(url).send().await {
        Ok(response) if response.status().is_success() => response,
        _ => return Vec::new(),
    };
    let payload = match response.json::<BrowserTaskList>().await {
        Ok(payload) => payload,
        Err(_) => return Vec::new(),
    };
    payload.items.into_iter().map(map_task).collect()
}

pub fn spawn_recover(browser_assist_url: String, payload: BrowserTaskPayload) {
    tokio::spawn(async move {
        let _ = submit_task(&browser_assist_url, "recover", payload).await;
    });
}

pub async fn submit_task(
    browser_assist_url: &str,
    kind: &str,
    payload: BrowserTaskPayload,
) -> Result<BrowserTask, StatusCode> {
    let url = format!(
        "{}/v1/tasks/{}",
        browser_assist_url.trim_end_matches('/'),
        kind
    );
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !response.status().is_success() {
        return Err(response.status());
    }
    let payload = response
        .json::<BrowserTaskEnvelope>()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    Ok(map_task(payload.task))
}

fn map_task(task: BrowserTaskWire) -> BrowserTask {
    BrowserTask {
        id: task.id,
        kind: task.kind,
        account_id: task.account_id,
        account_label: task.account_label,
        provider: task.provider,
        route_mode: task.route_mode,
        status: task.status,
        created_at: task.created_at,
        updated_at: task.updated_at,
        notes: task.notes,
        profile_dir: task.profile_dir,
        screenshot_path: task.screenshot_path,
        storage_state_path: task.storage_state_path,
        final_url: task.final_url,
        last_error: task.last_error,
        step_count: task.steps.len(),
    }
}
