//! Deterministic, local voice snippets resolved after model processing.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::OnceLock};

static PATH: OnceLock<PathBuf> = OnceLock::new();
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceSnippet {
    pub id: String,
    pub trigger: String,
    pub expansion: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub require_activation_phrase: bool,
}

fn default_true() -> bool {
    true
}

pub fn init(data_dir: PathBuf) {
    let _ = PATH.set(data_dir.join("snippets.json"));
    let _ = LOCK.set(Mutex::new(()));
}

fn read_unlocked() -> Result<Vec<VoiceSnippet>, String> {
    crate::storage::read_json(PATH.get().ok_or("Armazenamento indisponível")?)
}

pub fn list() -> Result<Vec<VoiceSnippet>, String> {
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    read_unlocked()
}

pub fn replace(snippets: Vec<VoiceSnippet>) -> Result<Vec<VoiceSnippet>, String> {
    let _config = crate::models::CONFIG_LOCK.lock();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    validate(&snippets)?;
    let path = PATH.get().ok_or("snippet storage is not initialized")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let json = serde_json::to_string_pretty(&snippets).map_err(|error| error.to_string())?;
    crate::storage::write_json(
        path,
        &serde_json::from_str::<serde_json::Value>(&json).map_err(|error| error.to_string())?,
    )?;
    Ok(snippets)
}

fn validate(snippets: &[VoiceSnippet]) -> Result<(), String> {
    let mut triggers = std::collections::HashSet::new();
    for snippet in snippets {
        let trigger = normalize(&snippet.trigger);
        if snippet.id.trim().is_empty() || trigger.is_empty() || snippet.expansion.is_empty() {
            return Err("snippet id, trigger and expansion must be non-empty".into());
        }
        if !triggers.insert(trigger) {
            return Err("snippet triggers must be unique".into());
        }
    }
    Ok(())
}

/// Expands snippets in the final text. A whole utterance equal to a trigger
/// expands to the snippet (unless it requires the activation phrase); inside a
/// sentence only the explicit "snippet <trigger>" / "expandir <trigger>" forms
/// expand, so ordinary speech never triggers by accident.
pub fn resolve(text: &str, snippets: &[VoiceSnippet]) -> Option<(String, String)> {
    let spoken = normalize(text.trim_matches(|c: char| matches!(c, '.' | '!' | '?' | ',' | ';')));
    let whole = snippets.iter().find_map(|snippet| {
        if !snippet.enabled {
            return None;
        }
        let trigger = normalize(&snippet.trigger);
        let explicit = format!("snippet {trigger}");
        let expand = format!("expandir {trigger}");
        let matches = if snippet.require_activation_phrase {
            spoken == explicit || spoken == expand
        } else {
            spoken == trigger || spoken == explicit || spoken == expand
        };
        matches.then(|| (snippet.expansion.clone(), snippet.id.clone()))
    });
    whole.or_else(|| expand_inline(text, snippets))
}

fn expand_inline(text: &str, snippets: &[VoiceSnippet]) -> Option<(String, String)> {
    let mut output = text.to_string();
    let mut used = Vec::new();
    let mut ordered = snippets
        .iter()
        .filter(|snippet| snippet.enabled && !snippet.trigger.trim().is_empty())
        .collect::<Vec<_>>();
    // Longest trigger first so "meu github pessoal" wins over "meu github".
    ordered.sort_by_key(|snippet| std::cmp::Reverse(snippet.trigger.chars().count()));
    for snippet in ordered {
        let trigger = normalize(&snippet.trigger);
        for prefix in ["snippet", "expandir"] {
            let phrase = format!("{prefix} {trigger}");
            while let Some((start, end)) = find_phrase(&output, &phrase) {
                // Swallow the punctuation a recognizer adds right after the phrase.
                let end = end
                    + output[end..]
                        .chars()
                        .take_while(|character| matches!(character, '.' | ','))
                        .map(char::len_utf8)
                        .sum::<usize>();
                output.replace_range(start..end, &snippet.expansion);
                if !used.contains(&snippet.id) {
                    used.push(snippet.id.clone());
                }
            }
        }
    }
    (!used.is_empty()).then(|| (output, used.join(",")))
}

/// Case-insensitive, whitespace-tolerant, word-bounded phrase search returning
/// byte offsets in `text`.
fn find_phrase(text: &str, phrase: &str) -> Option<(usize, usize)> {
    let words = phrase.split_whitespace().collect::<Vec<_>>();
    let tokens = text
        .char_indices()
        .filter(|(index, character)| {
            !character.is_whitespace()
                && text[..*index]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace)
        })
        .map(|(start, _)| {
            let end = text[start..]
                .find(char::is_whitespace)
                .map_or(text.len(), |offset| start + offset);
            (start, end)
        })
        .collect::<Vec<_>>();
    let clean = |value: &str| {
        value
            .trim_matches(|character: char| !character.is_alphanumeric())
            .to_lowercase()
    };
    tokens.windows(words.len()).find_map(|window| {
        let matches = window
            .iter()
            .zip(&words)
            .all(|((start, end), word)| clean(&text[*start..*end]) == clean(word));
        if !matches {
            return None;
        }
        let start = window[0].0;
        let last = window[words.len() - 1];
        // Keep trailing punctuation of the last word out of the match only
        // when it is not part of the phrase itself.
        let last_word = &text[last.0..last.1];
        let trimmed = last_word.trim_end_matches(|character: char| !character.is_alphanumeric());
        Some((start, last.0 + trimmed.len()))
    })
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github() -> VoiceSnippet {
        VoiceSnippet {
            id: "github".into(),
            trigger: "meu github".into(),
            expansion: "https://github.com/example".into(),
            enabled: true,
            require_activation_phrase: false,
        }
    }

    #[test]
    fn exact_trigger_expands_and_preserves_literal_value() {
        assert_eq!(
            resolve("Meu GitHub.", &[github()]).unwrap().0,
            "https://github.com/example"
        );
    }

    #[test]
    fn normal_sentence_is_not_a_false_positive() {
        assert!(resolve("acesse meu github quando puder", &[github()]).is_none());
    }

    #[test]
    fn explicit_phrase_expands_inside_a_sentence() {
        let (text, id) =
            resolve("Meu perfil é Snippet meu GitHub, pode olhar.", &[github()]).unwrap();
        assert_eq!(text, "Meu perfil é https://github.com/example pode olhar.");
        assert_eq!(id, "github");
        let (text, _) = resolve("veja expandir meu github", &[github()]).unwrap();
        assert_eq!(text, "veja https://github.com/example");
    }

    #[test]
    fn explicit_activation_can_be_required() {
        let mut snippet = github();
        snippet.require_activation_phrase = true;
        assert!(resolve("meu github", &[snippet.clone()]).is_none());
        assert!(resolve("snippet meu github", &[snippet]).is_some());
    }
}
