//! Short text-only LLM calls: the voice command ("rewrite the selection").
//!
//! Uses the cheapest fast model available with the user's keys: OpenRouter
//! first, then Google AI Studio.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::models::AppState;

const OPENROUTER_TEXT_MODEL: &str = "google/gemini-3.5-flash-lite";
const GOOGLE_TEXT_MODEL: &str = "gemini-3.5-flash-lite";

/// Persisted toggles for the optional assistance features.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureSettings {
    /// Offer to add repeated manual corrections to the vocabulary.
    #[serde(default = "default_true")]
    pub learning_prompts: bool,
    /// Check GitHub Releases on startup and install updates automatically.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Keep the microphone open between dictations so the first words are
    /// not lost while the device wakes up.
    #[serde(default)]
    pub mic_ready: crate::mic_ready::MicReadyMode,
}

fn default_true() -> bool {
    true
}

impl Default for FeatureSettings {
    fn default() -> Self {
        Self {
            learning_prompts: true,
            auto_update: true,
            mic_ready: crate::mic_ready::MicReadyMode::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LlmText {
    pub text: String,
    pub provider: &'static str,
    pub model: &'static str,
    pub duration_ms: u64,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

pub async fn text_llm(
    state: &AppState,
    system: &str,
    user: &str,
    timeout: Duration,
) -> Result<LlmText, String> {
    let mut last_error = None;
    if let Some(key) = state.next_openrouter_key() {
        match crate::openrouter::generate_text(system, user, OPENROUTER_TEXT_MODEL, &key, timeout)
            .await
        {
            Ok(out) => {
                return Ok(LlmText {
                    text: out.text,
                    provider: "openrouter",
                    model: OPENROUTER_TEXT_MODEL,
                    duration_ms: out.request_ms,
                    cost_usd: out.reported_cost_usd,
                    input_tokens: out.reported_input_tokens.map(|value| value as u64),
                    output_tokens: out.reported_output_tokens.map(|value| value as u64),
                })
            }
            Err(error) => last_error = Some(error),
        }
    }
    if let Some(key) = state.next_google_key() {
        let body = crate::gemini::client::GenerateContentRequest::with_parts(vec![
            crate::gemini::client::Part::Text {
                text: user.to_string(),
            },
        ])
        .with_system_instruction(system);
        match crate::gemini::client::generate_content_with_model(
            &key,
            GOOGLE_TEXT_MODEL,
            &body,
            timeout,
        )
        .await
        {
            Ok(out) => {
                return Ok(LlmText {
                    text: out.text.trim().to_string(),
                    provider: "google-ai-studio",
                    model: GOOGLE_TEXT_MODEL,
                    duration_ms: out.duration_ms,
                    cost_usd: None,
                    input_tokens: out.usage.input_tokens,
                    output_tokens: out.usage.output_tokens,
                })
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        "Configure uma chave do OpenRouter ou do Google em Provedores e APIs.".into()
    }))
}

const COMMAND_SYSTEM: &str = "Você é o modo comando de um app de ditado. O usuário falou uma \
instrução. Se houver [TEXTO SELECIONADO], reescreva esse texto seguindo a instrução. Se não \
houver, escreva o texto que a instrução pede. Devolva SOMENTE o texto final pronto para colar, \
sem aspas, títulos, comentários ou explicações. Mantenha o idioma do texto selecionado, a \
menos que a instrução peça outro.";

pub fn command_prompt(instruction: &str, selection: Option<&str>) -> (String, String) {
    let mut user = format!("[INSTRUÇÃO]\n{}", instruction.trim());
    if let Some(selection) = selection.filter(|value| !value.trim().is_empty()) {
        user.push_str("\n\n[TEXTO SELECIONADO]\n");
        user.push_str(selection);
    }
    (COMMAND_SYSTEM.to_string(), user)
}

pub async fn run_command(
    state: &AppState,
    instruction: &str,
    selection: Option<&str>,
) -> Result<LlmText, String> {
    let (system, user) = command_prompt(instruction, selection);
    text_llm(state, &system, &user, Duration::from_secs(45)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_prompt_includes_selection_only_when_present() {
        let (_, with) = command_prompt("deixa formal", Some("oi tudo bem"));
        assert!(with.contains("[TEXTO SELECIONADO]\noi tudo bem"));
        let (_, without) = command_prompt("escreva um e-mail", Some("  "));
        assert!(!without.contains("SELECIONADO"));
    }
}
