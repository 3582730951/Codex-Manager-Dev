use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct ReplayPack {
    pub cache_key: String,
    pub static_prefix_tokens: u64,
    pub workflow_tokens: u64,
    pub live_tail_tokens: u64,
    pub total_tokens: u64,
}

pub fn compile_replay_pack(
    principal_id: &str,
    model: &str,
    generation: u32,
    input: &Value,
) -> ReplayPack {
    let input_tokens = estimate_tokens(input);
    let static_prefix_tokens = 3072;
    let workflow_tokens = 768;
    let live_tail_tokens = input_tokens.max(96);
    let digest = Sha256::digest(format!("{principal_id}:{model}:{generation}").as_bytes());
    ReplayPack {
        cache_key: format!(
            "replay-{:02x}{:02x}{:02x}{:02x}-g{}",
            digest[0], digest[1], digest[2], digest[3], generation
        ),
        static_prefix_tokens,
        workflow_tokens,
        live_tail_tokens,
        total_tokens: static_prefix_tokens + workflow_tokens + live_tail_tokens,
    }
}

pub fn estimate_tokens(value: &Value) -> u64 {
    let bytes = serde_json::to_vec(value)
        .map(|value| value.len())
        .unwrap_or_default();
    ((bytes as f64) / 4.0).ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_for_same_inputs() {
        let input = serde_json::json!({"role":"user","content":"hello"});
        let a = compile_replay_pack("principal-1", "gpt-5.4", 3, &input);
        let b = compile_replay_pack("principal-1", "gpt-5.4", 3, &input);
        assert_eq!(a.cache_key, b.cache_key);
        assert_eq!(a.total_tokens, b.total_tokens);
    }
}
