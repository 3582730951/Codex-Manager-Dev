pub fn normalize_reasoning_effort(value: Option<&str>) -> Option<String> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "low" => Some("low".to_string()),
        "medium" => Some("medium".to_string()),
        "high" => Some("high".to_string()),
        "xhigh" | "extra_high" => Some("xhigh".to_string()),
        _ => None,
    }
}
