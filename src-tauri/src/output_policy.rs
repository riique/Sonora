//! Output policy resolved independently from the recognition provider.

use serde::{Deserialize, Serialize};

use crate::context::ContextSnapshot;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormattingLevel {
    Literal,
    #[default]
    Smart,
    Aggressive,
}

impl FormattingLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Literal => "literal",
            Self::Smart => "smart",
            Self::Aggressive => "aggressive",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationDestination {
    #[default]
    FocusedField,
    ClipboardOnly,
    Scratchpad,
}

impl DictationDestination {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FocusedField => "focused_field",
            Self::ClipboardOnly => "clipboard_only",
            Self::Scratchpad => "scratchpad",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMatcher {
    #[serde(default)]
    pub processes: Vec<String>,
    #[serde(default)]
    pub executables: Vec<String>,
    #[serde(default)]
    pub window_titles: Vec<String>,
    #[serde(default)]
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputProfile {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub matcher: ProfileMatcher,
    #[serde(default)]
    pub formatting_level: Option<FormattingLevel>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub style_instruction: Option<String>,
    #[serde(default)]
    pub allow_context_to_cloud: Option<bool>,
}

fn default_true() -> bool {
    true
}

impl Default for OutputProfile {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "Padrão".into(),
            enabled: true,
            matcher: ProfileMatcher::default(),
            // Inherit the global "Formatação" setting; a concrete value here would
            // silently override it for every dictation.
            formatting_level: None,
            content_type: None,
            style_instruction: None,
            allow_context_to_cloud: Some(false),
        }
    }
}

pub fn default_output_profiles() -> Vec<OutputProfile> {
    let mut codex = OutputProfile {
        id: "codex".into(),
        name: "Codex · programação".into(),
        content_type: Some("programming".into()),
        style_instruction: Some(
            "Preserve código, paths, comandos e identificadores literalmente.".into(),
        ),
        ..Default::default()
    };
    codex.matcher.processes = vec!["codex.exe".into()];
    codex.matcher.window_titles = vec!["Codex".into()];
    let mut chatgpt = OutputProfile {
        id: "chatgpt".into(),
        name: "ChatGPT · chat".into(),
        ..Default::default()
    };
    chatgpt.matcher.domains = vec!["chatgpt.com".into()];
    let mut gemini = OutputProfile {
        id: "gemini".into(),
        name: "Gemini · chat".into(),
        ..Default::default()
    };
    gemini.matcher.domains = vec!["gemini.google.com".into()];
    vec![OutputProfile::default(), codex, chatgpt, gemini]
}

/// Profiles saved before built-in styles inherited the global level carry an
/// explicit `Smart` that made the global "Formatação" setting a no-op.
pub fn migrate_builtin_formatting_levels(profiles: &mut [OutputProfile]) {
    for profile in profiles {
        if BUILTIN_PROFILE_IDS.contains(&profile.id.as_str())
            && profile.formatting_level == Some(FormattingLevel::Smart)
        {
            profile.formatting_level = None;
        }
    }
}

const BUILTIN_PROFILE_IDS: [&str; 4] = ["default", "codex", "chatgpt", "gemini"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedOutputProfile {
    pub profile_id: String,
    pub formatting_level: FormattingLevel,
    pub content_type: Option<String>,
    pub style_instruction: Option<String>,
    pub allow_context_to_cloud: bool,
}

pub fn resolve_output_profile(
    profiles: &[OutputProfile],
    context: &ContextSnapshot,
    temporary_override: Option<&str>,
    global_level: FormattingLevel,
) -> ResolvedOutputProfile {
    let default_profile = profiles
        .iter()
        .find(|profile| profile.enabled && profile.id == "default");
    let selected = temporary_override
        .and_then(|id| {
            profiles
                .iter()
                .find(|profile| profile.enabled && profile.id == id)
        })
        .or_else(|| {
            profiles
                .iter()
                .find(|profile| profile.enabled && domain_matches(profile, context))
        })
        .or_else(|| {
            profiles
                .iter()
                .find(|profile| profile.enabled && application_matches(profile, context))
        })
        .or(default_profile);

    let mut resolved = ResolvedOutputProfile {
        profile_id: "default".into(),
        formatting_level: global_level,
        content_type: None,
        style_instruction: None,
        allow_context_to_cloud: false,
    };
    if let Some(default_profile) = default_profile {
        apply_profile(&mut resolved, default_profile);
    }
    if let Some(selected) = selected {
        apply_profile(&mut resolved, selected);
    }
    resolved
}

fn apply_profile(resolved: &mut ResolvedOutputProfile, profile: &OutputProfile) {
    resolved.profile_id = profile.id.clone();
    if let Some(level) = profile.formatting_level {
        resolved.formatting_level = level;
    }
    if profile.content_type.is_some() {
        resolved.content_type = profile.content_type.clone();
    }
    if profile.style_instruction.is_some() {
        resolved.style_instruction = profile.style_instruction.clone();
    }
    if let Some(allow) = profile.allow_context_to_cloud {
        resolved.allow_context_to_cloud = allow;
    }
}

fn domain_matches(profile: &OutputProfile, context: &ContextSnapshot) -> bool {
    let Some(domain) = context.domain.as_deref().map(str::to_ascii_lowercase) else {
        return false;
    };
    profile.matcher.domains.iter().any(|candidate| {
        let candidate = candidate
            .trim()
            .trim_start_matches("*.")
            .to_ascii_lowercase();
        !candidate.is_empty() && (domain == candidate || domain.ends_with(&format!(".{candidate}")))
    })
}

fn application_matches(profile: &OutputProfile, context: &ContextSnapshot) -> bool {
    let process = context
        .process
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let executable = context
        .executable
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let title = context
        .window_title
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    profile
        .matcher
        .processes
        .iter()
        .any(|candidate| process == candidate.trim().to_ascii_lowercase())
        || profile.matcher.executables.iter().any(|candidate| {
            let candidate = candidate.trim().to_ascii_lowercase();
            executable == candidate || executable.ends_with(&format!("\\{candidate}"))
        })
        || profile.matcher.window_titles.iter().any(|candidate| {
            let candidate = candidate.trim().to_ascii_lowercase();
            !candidate.is_empty() && title.contains(&candidate)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, level: FormattingLevel) -> OutputProfile {
        OutputProfile {
            id: id.into(),
            name: id.into(),
            formatting_level: Some(level),
            ..Default::default()
        }
    }

    #[test]
    fn global_level_applies_to_builtin_profiles() {
        let mut profiles = default_output_profiles();
        let context = ContextSnapshot {
            domain: Some("chatgpt.com".into()),
            ..Default::default()
        };
        for global in [FormattingLevel::Literal, FormattingLevel::Aggressive] {
            assert_eq!(
                resolve_output_profile(&profiles, &ContextSnapshot::default(), None, global)
                    .formatting_level,
                global
            );
            assert_eq!(
                resolve_output_profile(&profiles, &context, None, global).formatting_level,
                global
            );
        }
        // Legacy persisted built-ins with an explicit Smart are migrated to inherit.
        for profile in &mut profiles {
            profile.formatting_level = Some(FormattingLevel::Smart);
        }
        migrate_builtin_formatting_levels(&mut profiles);
        assert_eq!(
            resolve_output_profile(&profiles, &context, None, FormattingLevel::Literal)
                .formatting_level,
            FormattingLevel::Literal
        );
    }

    #[test]
    fn precedence_is_override_then_domain_then_application_then_default() {
        let default = profile("default", FormattingLevel::Smart);
        let mut app = profile("codex", FormattingLevel::Literal);
        app.matcher.processes = vec!["codex.exe".into()];
        let mut domain = profile("chatgpt", FormattingLevel::Aggressive);
        domain.matcher.domains = vec!["chatgpt.com".into()];
        let context = ContextSnapshot {
            process: Some("codex.exe".into()),
            domain: Some("chatgpt.com".into()),
            ..Default::default()
        };
        let profiles = vec![default, app, domain];
        assert_eq!(
            resolve_output_profile(&profiles, &context, None, FormattingLevel::Smart).profile_id,
            "chatgpt"
        );
        assert_eq!(
            resolve_output_profile(&profiles, &context, Some("codex"), FormattingLevel::Smart)
                .profile_id,
            "codex"
        );
    }
}
