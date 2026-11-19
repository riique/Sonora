//! Short OpenRouter LLM calls: voice command ("rewrite the selection") and
//! pronunciation evaluation.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::models::AppState;

const OPENROUTER_TEXT_MODEL: &str = "openai/gpt-4o-mini";
const OPENROUTER_PRONUNCIATION_MODEL: &str = "openai/gpt-audio-mini";

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
    let api_key = state
        .next_openrouter_key()
        .ok_or_else(|| "Configure uma chave do OpenRouter em Ajustes → Transcrição.".to_string())?;
    let out =
        crate::openrouter::generate_text(system, user, OPENROUTER_TEXT_MODEL, &api_key, timeout)
            .await?;
    Ok(LlmText {
        text: out.text,
        provider: "openrouter",
        model: OPENROUTER_TEXT_MODEL,
        duration_ms: out.request_ms,
        cost_usd: out.reported_cost_usd,
        input_tokens: out.reported_input_tokens.map(|value| value as u64),
        output_tokens: out.reported_output_tokens.map(|value| value as u64),
    })
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

const PRONUNCIATION_SYSTEM: &str =
    "Você é um avaliador internacional de proficiência oral e comunicação. \
Responda em português do Brasil, em Markdown, sem introduções fora da estrutura pedida.";

pub fn pronunciation_prompt(transcript: &str) -> String {
    format!(
        "Analise o áudio como um avaliador internacional de proficiência oral e \
comunicação.\n\n\
Objetivo da avaliação:\n\
- medir inteligibilidade, pronúncia, fluência, ritmo, entonação, gramática \
oral, vocabulário, coesão, naturalidade, segurança e adequação ao contexto;\n\
- dar uma nota geral (0 a 10, com 1 casa decimal);\n\
- classificar o desempenho na escala internacional CEFR (A1, A2, B1, B2, C1, C2);\n\
- indicar a proximidade de fala nativa (0 a 100) e confiança da avaliação (baixa, média ou alta).\n\n\
Regras:\n\
1. Priorize o áudio como fonte principal e use a transcrição apenas como apoio.\n\
2. Se o áudio estiver curto demais, ruim, com ruído forte ou insuficiente, diga isso explicitamente e reduza a confiança.\n\
3. Não invente palavras, contexto ou sotaque que o áudio não sustente.\n\
4. Seja específico, direto, técnico e construtivo.\n\n\
Estrutura obrigatória:\n\
## Resumo Executivo\n\
## Placar\n\
## Forças\n\
## Pontos de Atenção\n\
## Pronúncia e Inteligibilidade\n\
## Fluência e Ritmo\n\
## Plano de Melhoria\n\
## Veredito Final\n\n\
Transcrição de referência (apoio):\n\"\"\"\n{}\n\"\"\"",
        transcript
    )
}

/// Evaluates oral pronunciation via OpenRouter's audio-capable model.
pub async fn evaluate_pronunciation(
    audio_bytes: &[u8],
    ext: &str,
    transcript: &str,
    openrouter_api_key: &str,
) -> Result<String, String> {
    if audio_bytes.is_empty() {
        return Err("áudio vazio; não é possível avaliar a pronúncia".to_string());
    }
    let prompt = pronunciation_prompt(transcript);
    let out = crate::openrouter::generate_with_audio(
        audio_bytes,
        ext,
        PRONUNCIATION_SYSTEM,
        &prompt,
        OPENROUTER_PRONUNCIATION_MODEL,
        openrouter_api_key,
        Duration::from_secs(90),
    )
    .await?;
    if out.text.trim().is_empty() {
        return Err("O modelo não retornou nenhum feedback de pronúncia.".into());
    }
    Ok(out.text)
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

    #[test]
    fn assist_uses_non_gemini_openrouter_models() {
        assert!(!OPENROUTER_TEXT_MODEL.contains("gemini"));
        assert!(!OPENROUTER_PRONUNCIATION_MODEL.contains("gemini"));
    }
}
