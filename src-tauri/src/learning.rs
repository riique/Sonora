//! Conservative local vocabulary learning from explicit user corrections.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::OnceLock};

static PATH: OnceLock<PathBuf> = OnceLock::new();
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionContext {
    #[serde(default)]
    pub application: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionSource {
    #[default]
    HistoryEdit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    #[default]
    Pending,
    Accepted,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionEvent {
    pub id: String,
    pub before: String,
    pub after: String,
    pub context: CorrectionContext,
    pub timestamp_ms: u64,
    pub count: u32,
    /// Per-occurrence timestamps enable period-aware Insights without changing
    /// the existing aggregate count. Older files migrate safely from an empty
    /// list and remain available in all-time statistics.
    #[serde(default)]
    pub occurrences_ms: Vec<u64>,
    pub source: CorrectionSource,
    #[serde(default)]
    pub status: SuggestionStatus,
}

pub fn init(data_dir: PathBuf) {
    let _ = PATH.set(data_dir.join("vocabulary-learning.json"));
    let _ = LOCK.set(Mutex::new(()));
}

fn read_unlocked() -> Result<Vec<CorrectionEvent>, String> {
    crate::storage::read_json(
        PATH.get()
            .ok_or("Armazenamento de correções indisponível")?,
    )
}

fn write_unlocked(events: &[CorrectionEvent]) -> Result<(), String> {
    let path = PATH
        .get()
        .ok_or("vocabulary learning storage is not initialized")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    crate::storage::write_json(path, events)
}

pub fn record(
    before_text: &str,
    after_text: &str,
    context: CorrectionContext,
) -> Result<Option<CorrectionEvent>, String> {
    let Some((before, after)) = localized_lexical_diff(before_text, after_text) else {
        return Ok(None);
    };
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut events = read_unlocked()?;
    let updated = upsert_event(&mut events, before, after, context);
    write_unlocked(&events)?;
    Ok(Some(updated))
}

fn upsert_event(
    events: &mut Vec<CorrectionEvent>,
    before: String,
    after: String,
    context: CorrectionContext,
) -> CorrectionEvent {
    if let Some(event) = events
        .iter_mut()
        .find(|event| event.before.eq_ignore_ascii_case(&before) && event.after == after)
    {
        let timestamp_ms = crate::pipeline_run::epoch_ms();
        event.count = event.count.saturating_add(1);
        event.timestamp_ms = timestamp_ms;
        event.occurrences_ms.push(timestamp_ms);
        event.context = context;
        event.clone()
    } else {
        let timestamp_ms = crate::pipeline_run::epoch_ms();
        let event = CorrectionEvent {
            id: format!("correction-{timestamp_ms}"),
            before,
            after,
            context,
            timestamp_ms,
            count: 1,
            occurrences_ms: vec![timestamp_ms],
            source: CorrectionSource::HistoryEdit,
            status: SuggestionStatus::Pending,
        };
        events.insert(0, event.clone());
        event
    }
}

pub fn suggestions() -> Vec<CorrectionEvent> {
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    read_unlocked()
        .unwrap_or_else(|error| {
            log::error!("learning: {error}");
            Vec::new()
        })
        .into_iter()
        .filter(is_suggestion)
        .collect()
}

/// Repeated corrections become vocabulary suggestions on the second occurrence.
pub const SUGGESTION_THRESHOLD: u32 = 2;

pub fn is_suggestion(event: &CorrectionEvent) -> bool {
    event.count >= SUGGESTION_THRESHOLD && event.status == SuggestionStatus::Pending
}

/// Read-only snapshot used by the local Insights projection. Correction text
/// never leaves the device unless the user explicitly generates an AI profile,
/// and that profile receives aggregate counts rather than these events.
pub fn all() -> Vec<CorrectionEvent> {
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    read_unlocked().unwrap_or_else(|error| {
        log::error!("learning: {error}");
        Vec::new()
    })
}

pub fn accept_pair(before: &str, after: &str) -> Result<(), String> {
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut events = read_unlocked()?;
    let mut changed = false;
    for event in &mut events {
        if event.before.eq_ignore_ascii_case(before) && event.after.eq_ignore_ascii_case(after) {
            event.status = SuggestionStatus::Accepted;
            changed = true;
        }
    }
    if changed {
        write_unlocked(&events)?;
    }
    Ok(())
}

pub fn resolve(id: &str, accepted: bool) -> Result<Option<CorrectionEvent>, String> {
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut events = read_unlocked()?;
    let result = events.iter_mut().find(|event| event.id == id).map(|event| {
        event.status = if accepted {
            SuggestionStatus::Accepted
        } else {
            SuggestionStatus::Dismissed
        };
        event.clone()
    });
    if result.is_some() {
        write_unlocked(&events)?;
    }
    Ok(result)
}

fn localized_lexical_diff(before: &str, after: &str) -> Option<(String, String)> {
    if before == after {
        return None;
    }
    let style_normalized = |value: &str| {
        value
            .chars()
            .filter(|character| character.is_alphanumeric() || character.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let canonical_case_change = |old: &str, new: &str| {
        old.split_whitespace().count() == 1
            && new.split_whitespace().count() == 1
            && old.eq_ignore_ascii_case(new)
            && (new
                .chars()
                .filter(|character| character.is_uppercase())
                .count()
                >= 2
                || (new.chars().next().is_some_and(char::is_lowercase)
                    && new.chars().any(char::is_uppercase)))
    };
    let before_words: Vec<&str> = before.split_whitespace().collect();
    let after_words: Vec<&str> = after.split_whitespace().collect();
    let canonical_case_only = before_words.len() == after_words.len()
        && before_words
            .iter()
            .zip(&after_words)
            .all(|(old, new)| old.eq_ignore_ascii_case(new))
        && before_words
            .iter()
            .zip(&after_words)
            .any(|(old, new)| canonical_case_change(old, new));
    if style_normalized(before) == style_normalized(after) && !canonical_case_only {
        return None;
    }
    let mut prefix = 0;
    while prefix < before_words.len()
        && prefix < after_words.len()
        && before_words[prefix] == after_words[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < before_words.len().saturating_sub(prefix)
        && suffix < after_words.len().saturating_sub(prefix)
        && before_words[before_words.len() - 1 - suffix]
            == after_words[after_words.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old = before_words[prefix..before_words.len().saturating_sub(suffix)].join(" ");
    let new = after_words[prefix..after_words.len().saturating_sub(suffix)].join(" ");
    if old.is_empty()
        || new.is_empty()
        || old.split_whitespace().count() > 4
        || new.split_whitespace().count() > 4
    {
        return None;
    }
    let lexical = |value: &str| {
        value.chars().any(char::is_alphanumeric)
            && value.chars().all(|character| {
                character.is_alphanumeric() || matches!(character, ' ' | '-' | '_' | '.' | '/')
            })
    };
    if !lexical(&old)
        || !lexical(&new)
        || (old.eq_ignore_ascii_case(&new) && !canonical_case_change(&old, &new))
    {
        return None;
    }
    Some((old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_edit_is_local_and_style_is_rejected() {
        assert_eq!(
            localized_lexical_diff("use open router aqui", "use OpenRouter aqui"),
            Some(("open router".into(), "OpenRouter".into()))
        );
        assert!(localized_lexical_diff("texto normal", "Texto normal.").is_none());
        assert!(localized_lexical_diff(
            "um texto curto",
            "reescrita completa com outra intenção e muitos detalhes"
        )
        .is_none());
        assert_eq!(
            localized_lexical_diff("use openrouter", "use OpenRouter"),
            Some(("openrouter".into(), "OpenRouter".into()))
        );
    }

    #[test]
    fn repeated_lexical_correction_reaches_suggestion_threshold() {
        let mut events = Vec::new();
        for _ in 0..3 {
            upsert_event(
                &mut events,
                "open router".into(),
                "OpenRouter".into(),
                CorrectionContext::default(),
            );
        }
        assert_eq!(events[0].count, 3);
        assert_eq!(events[0].occurrences_ms.len(), 3);
        assert_eq!(events[0].status, SuggestionStatus::Pending);
    }

    #[test]
    fn legacy_correction_migrates_without_occurrence_history() {
        let event: CorrectionEvent = serde_json::from_value(serde_json::json!({
            "id": "legacy",
            "before": "open router",
            "after": "OpenRouter",
            "context": {},
            "timestamp_ms": 1,
            "count": 4,
            "source": "history_edit",
            "status": "accepted"
        }))
        .unwrap();
        assert_eq!(event.count, 4);
        assert!(event.occurrences_ms.is_empty());
    }
}
