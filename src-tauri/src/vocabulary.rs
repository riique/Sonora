//! Structured vocabulary: terms, aliases, categories and strict literals.
//!
//! Migrates legacy `custom_words: string[]` without data loss.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Minimum categories required by product.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabularyCategory {
    AiModel,
    Provider,
    Application,
    Person,
    File,
    Command,
    Function,
    Identifier,
    StudyTerm,
    #[default]
    Other,
}

impl VocabularyCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AiModel => "ai_model",
            Self::Provider => "provider",
            Self::Application => "application",
            Self::Person => "person",
            Self::File => "file",
            Self::Command => "command",
            Self::Function => "function",
            Self::Identifier => "identifier",
            Self::StudyTerm => "study_term",
            Self::Other => "other",
        }
    }

    pub fn label_pt(self) -> &'static str {
        match self {
            Self::AiModel => "Modelo de IA",
            Self::Provider => "Provedor",
            Self::Application => "Aplicativo",
            Self::Person => "Pessoa",
            Self::File => "Arquivo",
            Self::Command => "Comando",
            Self::Function => "Função",
            Self::Identifier => "Identificador",
            Self::StudyTerm => "Termo de estudo",
            Self::Other => "Outro",
        }
    }

    pub fn all() -> &'static [VocabularyCategory] {
        &[
            Self::AiModel,
            Self::Provider,
            Self::Application,
            Self::Person,
            Self::File,
            Self::Command,
            Self::Function,
            Self::Identifier,
            Self::StudyTerm,
            Self::Other,
        ]
    }
}

/// One structured vocabulary entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyTerm {
    pub canonical: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub category: VocabularyCategory,
    /// When true, treat as a protected literal (deterministic alias→canonical).
    #[serde(default)]
    pub strict: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl VocabularyTerm {
    pub fn from_legacy_word(word: &str) -> Option<Self> {
        let canonical = word.trim().to_string();
        if canonical.is_empty() {
            return None;
        }
        Some(Self {
            canonical,
            aliases: Vec::new(),
            category: VocabularyCategory::Other,
            strict: false,
            enabled: true,
        })
    }
}

/// Migrate legacy string list → structured terms (no loss of non-empty uniques).
pub fn migrate_from_strings(words: &[String]) -> Vec<VocabularyTerm> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for w in words {
        let Some(term) = VocabularyTerm::from_legacy_word(w) else {
            continue;
        };
        let key = term.canonical.to_lowercase();
        if seen.insert(key) {
            out.push(term);
        }
    }
    out
}

/// Normalize, drop blanks, reject duplicate canonicals / conflicting aliases.
pub fn normalize_and_validate(terms: Vec<VocabularyTerm>) -> Result<Vec<VocabularyTerm>, String> {
    let mut cleaned = Vec::new();
    let mut canonical_keys: HashSet<String> = HashSet::new();
    // alias_lower -> owner canonical (for conflict detection)
    let mut alias_owners: HashMap<String, String> = HashMap::new();

    for mut t in terms {
        t.canonical = t.canonical.trim().to_string();
        if t.canonical.is_empty() {
            continue;
        }
        t.aliases = t
            .aliases
            .into_iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .filter(|a| !a.eq_ignore_ascii_case(&t.canonical))
            .collect();
        // dedupe aliases case-insensitively
        let mut a_seen = HashSet::new();
        t.aliases.retain(|a| a_seen.insert(a.to_lowercase()));

        let ckey = t.canonical.to_lowercase();
        if !canonical_keys.insert(ckey.clone()) {
            return Err(format!(
                "Termo duplicado: \"{}\" já existe no vocabulário.",
                t.canonical
            ));
        }

        // Canonical of one term must not be alias of another.
        if let Some(owner) = alias_owners.get(&ckey) {
            return Err(format!(
                "Conflito: \"{}\" é alias de \"{}\".",
                t.canonical, owner
            ));
        }

        for a in &t.aliases {
            let akey = a.to_lowercase();
            if canonical_keys.contains(&akey) && akey != ckey {
                return Err(format!(
                    "Conflito: o alias \"{}\" coincide com outro termo canônico.",
                    a
                ));
            }
            if let Some(owner) = alias_owners.get(&akey) {
                return Err(format!(
                    "Conflito: o alias \"{}\" já pertence a \"{}\".",
                    a, owner
                ));
            }
            alias_owners.insert(akey, t.canonical.clone());
        }

        cleaned.push(t);
    }
    Ok(cleaned)
}

/// Enabled terms only.
pub fn enabled_terms(terms: &[VocabularyTerm]) -> Vec<&VocabularyTerm> {
    terms.iter().filter(|t| t.enabled).collect()
}

/// Canonical spellings for simple prompt lists (enabled only).
pub fn canonical_list(terms: &[VocabularyTerm]) -> Vec<String> {
    enabled_terms(terms)
        .into_iter()
        .map(|t| t.canonical.clone())
        .collect()
}

/// Product hint for recognizers. "sonora" is also an ordinary Portuguese
/// adjective, so do not force replacements in phrases such as "trilha sonora".
pub fn default_sonora_term() -> VocabularyTerm {
    VocabularyTerm {
        canonical: "Sonora".into(),
        aliases: vec![],
        category: VocabularyCategory::Application,
        strict: false,
        enabled: true,
    }
}

/// Add the product hint without overwriting user vocabulary or disabled terms.
pub fn ensure_default_product_terms(mut terms: Vec<VocabularyTerm>) -> Vec<VocabularyTerm> {
    let needle = "sonora";
    let has = terms
        .iter()
        .any(|t| t.canonical.eq_ignore_ascii_case(needle));
    if !has {
        // Keep legacy product terms as the user saved them.
        terms.insert(0, default_sonora_term());
    } else {
        // Merge missing aliases into existing term if user already has "Sonora".
        if let Some(t) = terms
            .iter_mut()
            .find(|t| t.canonical.eq_ignore_ascii_case(needle))
        {
            let defaults = default_sonora_term();
            for a in defaults.aliases {
                if !t.aliases.iter().any(|x| x.eq_ignore_ascii_case(&a)) {
                    t.aliases.push(a);
                }
            }
        }
    }
    terms
}

/// Format glossary block for LLM prompts (sanitizer / Gemini).
/// Short spelling hint for Whisper-style STT `prompt` fields. Whisper only reads the
/// tail (~224 tokens) of the prompt, so the list is capped and strict terms go first.
pub fn whisper_prompt_hint(terms: &[VocabularyTerm]) -> Option<String> {
    const MAX_CHARS: usize = 600;
    let mut ordered = terms
        .iter()
        .filter(|term| term.enabled && !term.canonical.trim().is_empty())
        .collect::<Vec<_>>();
    ordered.sort_by_key(|term| !term.strict);
    let mut hint = String::new();
    for term in ordered {
        let canonical = term.canonical.trim();
        let separator = if hint.is_empty() { "" } else { ", " };
        if hint.chars().count() + separator.len() + canonical.chars().count() > MAX_CHARS {
            break;
        }
        hint.push_str(separator);
        hint.push_str(canonical);
    }
    (!hint.is_empty()).then(|| format!("{hint}."))
}

pub fn format_glossary_for_prompt(terms: &[VocabularyTerm]) -> String {
    let enabled = enabled_terms(terms);
    if enabled.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    for t in enabled {
        let mut line = format!("- {} [{}]", t.canonical, t.category.as_str());
        if !t.aliases.is_empty() {
            line.push_str(&format!(" (aliases: {})", t.aliases.join(", ")));
        }
        if t.strict {
            line.push_str(" [LITERAL]");
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Deterministic post-pass: replace unambiguous aliases with canonical for
/// **strict + enabled** terms only. Case-insensitive; supports multi-word aliases.
/// Does not rewrite free text globally or force weak matches.
pub fn apply_strict_literals(text: &str, terms: &[VocabularyTerm]) -> (String, Vec<String>) {
    if text.is_empty() {
        return (text.to_string(), Vec::new());
    }

    // alias/canonical → canonical (longest first so multi-word wins).
    let mut pairs: Vec<(String, String)> = Vec::new();
    for t in terms.iter().filter(|t| t.enabled && t.strict) {
        for a in &t.aliases {
            if !a.trim().is_empty() {
                pairs.push((a.clone(), t.canonical.clone()));
            }
        }
        // Also normalize accidental wrong casing of the canonical itself.
        pairs.push((t.canonical.clone(), t.canonical.clone()));
    }
    pairs.sort_by_key(|pair| std::cmp::Reverse(pair.0.chars().count()));
    pairs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));

    if pairs.is_empty() {
        return (text.to_string(), Vec::new());
    }

    let mut applied = Vec::new();
    let mut out = text.to_string();

    for (from, to) in &pairs {
        let (next, hits) = replace_phrase_ci(&out, from, to);
        if hits > 0 && from.to_lowercase() != to.to_lowercase() {
            applied.push(format!("{}→{} (×{})", from, to, hits));
        }
        out = next;
    }

    (out, applied)
}

/// Case-insensitive whole-phrase replace. Phrase must be bounded by non-token
/// chars (or string edges) so we do not rewrite inside larger identifiers.
fn replace_phrase_ci(haystack: &str, needle: &str, replacement: &str) -> (String, usize) {
    if needle.is_empty() {
        return (haystack.to_string(), 0);
    }
    let h_lower: Vec<char> = haystack.to_lowercase().chars().collect();
    let n_lower: Vec<char> = needle.to_lowercase().chars().collect();
    let h_chars: Vec<char> = haystack.chars().collect();
    let n_len = n_lower.len();
    if n_len == 0 || h_chars.len() < n_len {
        return (haystack.to_string(), 0);
    }

    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    let mut hits = 0usize;
    while i < h_chars.len() {
        if i + n_len <= h_chars.len()
            && h_lower[i..i + n_len] == n_lower[..]
            && is_boundary_before(&h_chars, i)
            && is_boundary_after(&h_chars, i + n_len)
        {
            out.push_str(replacement);
            i += n_len;
            hits += 1;
        } else {
            out.push(h_chars[i]);
            i += 1;
        }
    }
    (out, hits)
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == '\\'
}

fn is_boundary_before(chars: &[char], i: usize) -> bool {
    i == 0 || !is_token_char(chars[i - 1])
}

fn is_boundary_after(chars: &[char], i: usize) -> bool {
    i >= chars.len() || !is_token_char(chars[i])
}

/// Detect if a strict canonical present in `before` was corrupted in `after`
/// (removed or replaced). Returns list of canonicals that look damaged.
pub fn detect_strict_corruption(
    before: &str,
    after: &str,
    terms: &[VocabularyTerm],
) -> Vec<String> {
    let after_lower = after.to_lowercase();
    let mut damaged = Vec::new();
    for t in terms.iter().filter(|t| t.enabled && t.strict) {
        let c = t.canonical.to_lowercase();
        let before_has = before.to_lowercase().contains(&c)
            || t.aliases
                .iter()
                .any(|a| before.to_lowercase().contains(&a.to_lowercase()));
        if !before_has {
            continue;
        }
        // After should contain canonical (strict protection target).
        if !after_lower.contains(&c) {
            damaged.push(t.canonical.clone());
        }
    }
    damaged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_legacy_strings_no_loss() {
        let words = vec![
            "Sonora".into(),
            "  ".into(),
            "Tokio".into(),
            "sonora".into(), // dup case
        ];
        let terms = migrate_from_strings(&words);
        assert_eq!(terms.len(), 2);
        assert_eq!(terms[0].canonical, "Sonora");
        assert_eq!(terms[1].canonical, "Tokio");
        assert!(terms.iter().all(|t| t.enabled && !t.strict));
    }

    #[test]
    fn reject_duplicate_canonical() {
        let terms = vec![
            VocabularyTerm {
                canonical: "Foo".into(),
                aliases: vec![],
                category: VocabularyCategory::Other,
                strict: false,
                enabled: true,
            },
            VocabularyTerm {
                canonical: "foo".into(),
                aliases: vec![],
                category: VocabularyCategory::File,
                strict: true,
                enabled: true,
            },
        ];
        assert!(normalize_and_validate(terms).is_err());
    }

    #[test]
    fn reject_alias_conflict() {
        let terms = vec![
            VocabularyTerm {
                canonical: "provider-routing.json".into(),
                aliases: vec!["provider routing json".into()],
                category: VocabularyCategory::File,
                strict: true,
                enabled: true,
            },
            VocabularyTerm {
                canonical: "Other".into(),
                aliases: vec!["provider routing json".into()],
                category: VocabularyCategory::Other,
                strict: false,
                enabled: true,
            },
        ];
        assert!(normalize_and_validate(terms).is_err());
    }

    #[test]
    fn strict_literal_replaces_alias_only() {
        let terms = vec![VocabularyTerm {
            canonical: "provider-routing.json".into(),
            aliases: vec!["provider routing json".into()],
            category: VocabularyCategory::File,
            strict: true,
            enabled: true,
        }];
        let (out, applied) = apply_strict_literals("Abra o provider routing json agora", &terms);
        assert!(out.contains("provider-routing.json"));
        assert!(!out.to_lowercase().contains("provider routing json"));
        assert!(!applied.is_empty());
    }

    #[test]
    fn sonora_aliases_and_code_preserved() {
        let terms = vec![default_sonora_term()];
        let (a, _) = apply_strict_literals("Ouça a trilha sonora agora", &terms);
        assert_eq!(a, "Ouça a trilha sonora agora");
        let (b, _) = apply_strict_literals("Sonora", &terms);
        assert_eq!(b, "Sonora");
        let (c, _) = apply_strict_literals("Homeia Voice e HowMeia Voice", &terms);
        assert!(
            c.to_lowercase().contains("homeia"),
            "Legacy names must not be rewritten as Sonora"
        );
        assert!(c.to_lowercase().contains("howmeia"));
        // Must not rewrite code-like tokens without alias match.
        let code = "provider-routing.json provider.only allow_fallbacks sanitizeTranscript()";
        let (d, _) = apply_strict_literals(code, &terms);
        assert_eq!(d, code);
    }

    #[test]
    fn non_strict_does_not_auto_replace() {
        let terms = vec![VocabularyTerm {
            canonical: "Sonora".into(),
            aliases: vec!["HowMeia".into()],
            category: VocabularyCategory::Application,
            strict: false,
            enabled: true,
        }];
        let (out, applied) = apply_strict_literals("HowMeia voice", &terms);
        assert_eq!(out, "HowMeia voice");
        assert!(applied.is_empty());
    }

    #[test]
    fn disabled_term_ignored() {
        let terms = vec![VocabularyTerm {
            canonical: "X".into(),
            aliases: vec!["Y".into()],
            category: VocabularyCategory::Other,
            strict: true,
            enabled: false,
        }];
        let (out, _) = apply_strict_literals("Y test", &terms);
        assert_eq!(out, "Y test");
        assert!(canonical_list(&terms).is_empty());
    }

    #[test]
    fn detect_corruption_when_canonical_vanishes() {
        let terms = vec![VocabularyTerm {
            canonical: "useEffect".into(),
            aliases: vec![],
            category: VocabularyCategory::Function,
            strict: true,
            enabled: true,
        }];
        let d = detect_strict_corruption("chame useEffect", "chame o efeito", &terms);
        assert_eq!(d, vec!["useEffect"]);
    }

    #[test]
    fn whisper_hint_lists_enabled_terms_strict_first_and_is_capped() {
        let term = |canonical: &str, strict: bool, enabled: bool| VocabularyTerm {
            canonical: canonical.into(),
            aliases: Vec::new(),
            category: VocabularyCategory::default(),
            strict,
            enabled,
        };
        assert_eq!(
            whisper_prompt_hint(&[
                term("Tauri", false, true),
                term("Sonora", true, true),
                term("Off", false, false),
            ])
            .as_deref(),
            Some("Sonora, Tauri.")
        );
        assert_eq!(whisper_prompt_hint(&[]), None);
        let many = (0..500)
            .map(|index| term(&format!("Termo{index}"), false, true))
            .collect::<Vec<_>>();
        assert!(whisper_prompt_hint(&many).unwrap().chars().count() <= 601);
    }

    #[test]
    fn glossary_format_includes_literal_flag() {
        let terms = vec![VocabularyTerm {
            canonical: "gsk_key".into(),
            aliases: vec!["gsk key".into()],
            category: VocabularyCategory::Identifier,
            strict: true,
            enabled: true,
        }];
        let g = format_glossary_for_prompt(&terms);
        assert!(g.contains("LITERAL"));
        assert!(g.contains("gsk_key"));
        assert!(g.contains("identifier"));
    }

    #[test]
    fn roundtrip_json_term() {
        let t = VocabularyTerm {
            canonical: "provider-routing.json".into(),
            aliases: vec!["provider routing json".into()],
            category: VocabularyCategory::File,
            strict: true,
            enabled: true,
        };
        let j = serde_json::to_string(&t).unwrap();
        let back: VocabularyTerm = serde_json::from_str(&j).unwrap();
        assert_eq!(back, t);
    }
}
