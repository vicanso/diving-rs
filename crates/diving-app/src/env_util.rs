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

/// Expands braced `${VAR}` and `${VAR:-default}` references using process environment variables.
///
/// Bare `$VAR` tokens (such as Harbor robot names `robot$ci`) are preserved verbatim.
pub fn expand_env_vars(raw: &str) -> String {
    expand_env_vars_with(raw, |key| std::env::var(key).ok())
}

/// Deterministic variant of [`expand_env_vars`] accepting a custom lookup function.
pub fn expand_env_vars_with<F>(raw: &str, lookup: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    if !raw.contains("${") {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str("${");
            rest = after;
            break;
        };
        let expr = &after[..end];
        let (var_name, default_val) = match expr.split_once(":-") {
            Some((name, fallback)) => (name, Some(fallback)),
            None => (expr, None),
        };
        let valid_ident = !var_name.is_empty()
            && var_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if valid_ident {
            match default_val {
                Some(fallback) => match lookup(var_name) {
                    Some(v) if !v.is_empty() => out.push_str(&v),
                    _ => out.push_str(fallback),
                },
                None => {
                    if let Some(v) = lookup(var_name) {
                        out.push_str(&v);
                    }
                }
            }
        } else {
            out.push_str("${");
            out.push_str(expr);
            out.push('}');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_env_vars_expands_braced_vars_and_preserves_bare_dollar() {
        let lookup = |k: &str| match k {
            "PASS" => Some("s3cret!".to_string()),
            "EMPTY" => Some(String::new()),
            _ => None,
        };
        assert_eq!(
            expand_env_vars_with("robot$prod+ci", lookup),
            "robot$prod+ci"
        );
        assert_eq!(expand_env_vars_with("$2a$10$hash", lookup), "$2a$10$hash");
        assert_eq!(
            expand_env_vars_with("Bearer ${PASS}", lookup),
            "Bearer s3cret!"
        );
        assert_eq!(
            expand_env_vars_with("${MISSING:-fallback}:${EMPTY:-def}:${MISSING}", lookup),
            "fallback:def:"
        );
        assert_eq!(expand_env_vars_with("${unclosed", lookup), "${unclosed");
    }
}
