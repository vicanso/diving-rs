/// Return the first non-empty trimmed value from `explicit` or env var `env_key`.
pub fn first_non_empty(explicit: Option<&str>, env_key: &str) -> Option<String> {
    if let Some(v) = explicit {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Ok(v) = std::env::var(env_key) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}
