//! Model → harness routing: which harness a model name belongs to, and whether
//! the harness a chat runs on can serve the model it was handed.
//!
//! Two consumers:
//!
//! * `orx model <name>` resolves an alias or id to the canonical model id and
//!   the harness that natively serves it (`opus` → `claude-code`,
//!   `gpt-5.5` → `codex`, `gemini-3.1-pro-high` → `antigravity`).
//! * Chat dispatch calls [`check_model_for_harness`] before a turn is queued
//!   and again before `run_turn`, so a model meant for one harness is never
//!   passed as `--model` to another (e.g. a GPT id to Claude Code).
//!
//! The check only rejects models it *recognizes* as another vendor's. Unknown
//! ids (local models, proxies, custom providers) pass through untouched, and
//! multi-provider harnesses (OpenCode, Cursor, ACP agents) accept every family.

use std::fmt;

/// The vendor family a model id belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    Claude,
    OpenAi,
    Gemini,
    Grok,
    DeepSeek,
    /// Cursor's own models (`composer-*`).
    Cursor,
}

impl ModelFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::OpenAi => "OpenAI",
            Self::Gemini => "Gemini",
            Self::Grok => "Grok",
            Self::DeepSeek => "DeepSeek",
            Self::Cursor => "Cursor",
        }
    }

    /// The harness that serves this family natively: the one `orx model`
    /// reports and the one a mismatch error points to.
    pub fn home_harness(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::OpenAi => "codex",
            Self::Gemini => "antigravity",
            Self::Grok | Self::Cursor => "cursor",
            Self::DeepSeek => "opencode",
        }
    }
}

/// Provider prefixes OpenCode-style ids carry (`anthropic/claude-sonnet-4-6`).
const VENDOR_PREFIXES: &[&str] = &[
    "anthropic/",
    "openai/",
    "google/",
    "xai/",
    "deepseek/",
    "cursor/",
];

/// Claude Code's own picker aliases; passed to `--model` as-is.
const CLAUDE_ALIASES: &[&str] = &["opus", "sonnet", "haiku", "fable", "opusplan"];

/// Ids every harness treats as "use your own default".
const DEFAULT_IDS: &[&str] = &["", "default", "auto"];

/// Display name for a harness id, without touching the registry (so this
/// module stays pure and cheap to call on every turn).
pub fn harness_display_name(harness: &str) -> &str {
    match harness {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "antigravity" => "Google Antigravity",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        other => other,
    }
}

/// `input` lowercased, trimmed, with inner whitespace turned into `-`
/// (`"Claude Opus 4.8"` → `"claude-opus-4.8"`).
fn normalize(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase()
}

/// `id` without a known vendor prefix.
fn strip_vendor(id: &str) -> &str {
    VENDOR_PREFIXES
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .unwrap_or(id)
}

/// The alias part of a Claude id, ignoring a context suffix (`opus[1m]`).
fn claude_alias(id: &str) -> Option<&str> {
    let base = id.split('[').next().unwrap_or(id);
    CLAUDE_ALIASES.contains(&base).then_some(base)
}

/// `o1`, `o3-mini`, `o4-mini-high` — OpenAI's reasoning series.
fn is_openai_o_series(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next() == Some('o')
        && chars.next().is_some_and(|c| c.is_ascii_digit())
        && chars.next().is_none_or(|c| c == '-' || c.is_ascii_digit())
}

/// The family of `model`, or `None` when it is a default sentinel or an id
/// this module does not recognize.
pub fn family_of(model: &str) -> Option<ModelFamily> {
    let normalized = normalize(model);
    let id = strip_vendor(&normalized);
    if DEFAULT_IDS.contains(&id) {
        return None;
    }
    if id.starts_with("claude") || claude_alias(id).is_some() {
        return Some(ModelFamily::Claude);
    }
    if id.starts_with("gpt") || id.starts_with("codex") || is_openai_o_series(id) {
        return Some(ModelFamily::OpenAi);
    }
    if id.starts_with("gemini") {
        return Some(ModelFamily::Gemini);
    }
    if id.starts_with("grok") {
        return Some(ModelFamily::Grok);
    }
    if id.starts_with("deepseek") {
        return Some(ModelFamily::DeepSeek);
    }
    if id.starts_with("composer") {
        return Some(ModelFamily::Cursor);
    }
    None
}

/// Whether `harness` can serve models of `family`. Single-vendor CLIs are
/// strict; multi-provider harnesses accept everything.
pub fn harness_serves(harness: &str, family: ModelFamily) -> bool {
    use ModelFamily::*;
    match harness {
        "claude-code" => family == Claude,
        "codex" => family == OpenAi,
        // Antigravity's catalog mixes Gemini with Claude and GPT-OSS models.
        "antigravity" => matches!(family, Gemini | Claude | OpenAi),
        // Cursor, OpenCode, ACP agents and anything unknown: multi-provider.
        _ => true,
    }
}

/// A model name resolved to its canonical id and home harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    /// The id to pass as `--model` on `harness`.
    pub model_id: String,
    pub family: ModelFamily,
    pub harness: &'static str,
}

/// The canonical id of `model` for `harness`: OpenCode keeps (or gains) the
/// `provider/` prefix it needs; every other harness takes the bare id.
fn id_for_harness(normalized: &str, family: ModelFamily, harness: &str) -> String {
    let bare = strip_vendor(normalized);
    // `claude-opus` / `claude-sonnet` are how people say the alias.
    let bare = bare
        .strip_prefix("claude-")
        .filter(|rest| claude_alias(rest).is_some())
        .unwrap_or(bare);
    // `gpt5.5` → `gpt-5.5`.
    let bare = match bare.strip_prefix("gpt") {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => format!("gpt-{rest}"),
        _ => bare.to_string(),
    };
    if harness != "opencode" {
        return bare;
    }
    let provider = match family {
        ModelFamily::Claude => "anthropic",
        ModelFamily::OpenAi => "openai",
        ModelFamily::Gemini => "google",
        ModelFamily::Grok => "xai",
        ModelFamily::DeepSeek => "deepseek",
        ModelFamily::Cursor => return bare,
    };
    format!("{provider}/{bare}")
}

/// Resolve an alias or id to its canonical model id and native harness.
pub fn resolve_model(input: &str) -> Result<ResolvedModel, String> {
    let normalized = normalize(input);
    if DEFAULT_IDS.contains(&strip_vendor(&normalized)) {
        return Err(format!(
            "`{}` means \"the harness default\", not a specific model; name a model such as `opus`, `gpt-5.5` or `gemini-3.1-pro-high`.",
            input.trim()
        ));
    }
    let family = family_of(&normalized).ok_or_else(|| {
        format!(
            "Unrecognized model `{}`: expected a Claude (claude-*, opus, sonnet, haiku, fable), OpenAI (gpt-*, o*, codex-*), Gemini (gemini-*), Grok (grok-*), DeepSeek (deepseek-*) or Cursor (composer-*) model.",
            input.trim()
        )
    })?;
    let harness = family.home_harness();
    Ok(ResolvedModel {
        model_id: id_for_harness(&normalized, family, harness),
        family,
        harness,
    })
}

/// A model handed to a harness that cannot serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMismatch {
    pub harness: String,
    pub model: String,
    pub family: ModelFamily,
}

impl fmt::Display for ModelMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let home = self.family.home_harness();
        write!(
            f,
            "Model `{}` belongs to {} and cannot run on {} ({}). Switch the chat to {} ({}) or pick a model from {}'s own list.",
            self.model,
            self.family.label(),
            harness_display_name(&self.harness),
            self.harness,
            harness_display_name(home),
            home,
            harness_display_name(&self.harness),
        )
    }
}

impl std::error::Error for ModelMismatch {}

/// Check that `harness` can serve `model`. Unknown models and default
/// sentinels pass; a recognized model of a family the harness does not serve
/// is a [`ModelMismatch`].
pub fn check_model_for_harness(harness: &str, model: &str) -> Result<(), ModelMismatch> {
    match family_of(model) {
        Some(family) if !harness_serves(harness, family) => Err(ModelMismatch {
            harness: harness.to_string(),
            model: model.trim().to_string(),
            family,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(input: &str) -> (String, &'static str) {
        let r = resolve_model(input).unwrap_or_else(|e| panic!("{input}: {e}"));
        (r.model_id, r.harness)
    }

    #[test]
    fn resolves_aliases_and_ids_to_their_home_harness() {
        let cases = [
            ("opus", "opus", "claude-code"),
            ("Claude Opus", "opus", "claude-code"),
            ("opus[1m]", "opus[1m]", "claude-code"),
            ("claude-haiku-4-5", "claude-haiku-4-5", "claude-code"),
            (
                "anthropic/claude-sonnet-4-6",
                "claude-sonnet-4-6",
                "claude-code",
            ),
            ("gpt-5.6-sol", "gpt-5.6-sol", "codex"),
            ("GPT5.5", "gpt-5.5", "codex"),
            ("o3-mini", "o3-mini", "codex"),
            ("codex-mini-latest", "codex-mini-latest", "codex"),
            ("gemini-3.1-pro-high", "gemini-3.1-pro-high", "antigravity"),
            ("grok-4.6", "grok-4.6", "cursor"),
            ("composer-1", "composer-1", "cursor"),
            (
                "deepseek-v4-flash",
                "deepseek/deepseek-v4-flash",
                "opencode",
            ),
        ];
        for (input, id, harness) in cases {
            assert_eq!(resolved(input), (id.to_string(), harness), "{input}");
        }
    }

    #[test]
    fn unknown_and_default_names_do_not_resolve() {
        assert!(resolve_model("default").is_err());
        assert!(resolve_model("auto").is_err());
        assert!(resolve_model("my-local-llama")
            .unwrap_err()
            .contains("Unrecognized model"));
        // `opera` and `o` must not look like Claude's `opus` or OpenAI's o-series.
        assert_eq!(family_of("opera"), None);
        assert_eq!(family_of("o"), None);
        assert_eq!(family_of("ollama/qwen3"), None);
    }

    #[test]
    fn single_vendor_harnesses_reject_other_vendors_models() {
        let err = check_model_for_harness("claude-code", "gpt-5.5").unwrap_err();
        assert_eq!(err.family, ModelFamily::OpenAi);
        let message = err.to_string();
        assert!(message.contains("Claude Code"), "{message}");
        assert!(message.contains("Codex (codex)"), "{message}");

        assert!(check_model_for_harness("codex", "opus").is_err());
        assert!(check_model_for_harness("codex", "gemini-3.1-pro-high").is_err());
        assert!(check_model_for_harness("antigravity", "grok-4.6").is_err());
    }

    #[test]
    fn matching_unknown_and_multi_provider_models_pass() {
        assert!(check_model_for_harness("claude-code", "opus[1m]").is_ok());
        assert!(check_model_for_harness("claude-code", "claude-haiku-4-5").is_ok());
        assert!(check_model_for_harness("codex", "gpt-5.6-sol").is_ok());
        assert!(check_model_for_harness("antigravity", "claude-sonnet-4-6").is_ok());
        assert!(check_model_for_harness("cursor", "claude-opus-4-8[effort=high]").is_ok());
        assert!(check_model_for_harness("opencode", "google/gemini-2.5-flash").is_ok());
        assert!(check_model_for_harness("acp:zcode", "gpt-5.5").is_ok());
        // Proxies and local models are not ours to judge.
        assert!(check_model_for_harness("claude-code", "glm-4.6").is_ok());
        assert!(check_model_for_harness("codex", "default").is_ok());
    }
}
