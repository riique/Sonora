//! Output policy attached to every dictation. Per-app styles and alternative
//! destinations were removed: every dictation uses Smart formatting and is
//! pasted into the focused field. The enums keep their old variants so
//! history written by earlier versions still loads.

use serde::{Deserialize, Serialize};

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
    /// Legacy "save as note" destination; delivered like `FocusedField`.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedOutputProfile {
    pub profile_id: String,
    pub formatting_level: FormattingLevel,
    pub content_type: Option<String>,
    pub style_instruction: Option<String>,
    pub allow_context_to_cloud: bool,
}

impl Default for ResolvedOutputProfile {
    fn default() -> Self {
        Self {
            profile_id: "default".into(),
            formatting_level: FormattingLevel::Smart,
            content_type: None,
            style_instruction: None,
            allow_context_to_cloud: false,
        }
    }
}
