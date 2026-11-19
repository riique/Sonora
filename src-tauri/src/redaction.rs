//! Redact credentials before errors enter logs, persistence or IPC.
static KEYS: parking_lot::RwLock<Vec<String>> = parking_lot::RwLock::new(Vec::new());
pub fn register(keys: &crate::models::ApiKeys) {
    *KEYS.write() = [
        &keys.groq,
        &keys.deepgram,
        &keys.openrouter,
        &keys.assemblyai,
        &keys.meta,
    ]
    .into_iter()
    .flatten()
    .filter(|key| !key.is_empty())
    .cloned()
    .collect();
}
pub fn message(value: &str) -> String {
    let mut result = value.to_string();
    for key in KEYS.read().iter() {
        result = result.replace(key, "[redacted]");
    }
    for marker in ["?key=", "&key=", "api_key=", "Bearer "] {
        let mut offset = 0;
        while let Some(index) = result[offset..].find(marker) {
            let start = offset + index + marker.len();
            let end = result[start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '&' | '"' | '\'' | ')' | '>'))
                .map_or(result.len(), |index| start + index);
            result.replace_range(start..end, "[redacted]");
            offset = start + "[redacted]".len();
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_query_and_bearer_errors_do_not_retain_credentials() {
        let text = message("request failed https://example.invalid/path?key=synthetic-secret&mode=1 Bearer synthetic-other");
        assert!(!text.contains("synthetic-secret"));
        assert!(!text.contains("synthetic-other"));
        assert!(text.contains("mode=1"));
    }
}
