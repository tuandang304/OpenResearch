//! Sub-agent limits (Settings → Harnesses → Sub-agent limits): the harnesses,
//! models and reasoning effort that chats an agent delegates to may use.
//!
//! Without limits an agent picks whatever harness and model it likes for
//! delegated work, sometimes one that is slow, out of quota, or not even served
//! by that harness, and the work stalls. With limits on:
//!
//! * a harness the user turned off is refused when named with `--harness`, and
//!   skipped in favour of an allowed one when the New Chat agent is off;
//! * a model outside the harness's allowed list, or one that belongs to another
//!   vendor's harness, is replaced by the harness's default sub-agent model;
//! * a fixed effort replaces whatever effort the agent asked for.
//!
//! Every substitution is reported back to the agent as a note, and the playbook
//! lists the limits so the agent picks an allowed combination the first time.
//! The limits apply only to chats agents create; the user's own chats, the
//! composer and harness failover are unaffected. One exception the user opts
//! into per harness: [`HarnessRule::subagent_model`] pins the model Claude Code
//! uses for its own built-in subagents (`CLAUDE_CODE_SUBAGENT_MODEL`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::harness::REASONING_DEFAULT_ID;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationPolicy {
    /// Off = no limits at all, whatever the rules say.
    #[serde(default)]
    pub enabled: bool,
    /// One entry per harness the user configured. A harness without an entry
    /// is allowed with no model or effort limits.
    #[serde(default)]
    pub harnesses: Vec<HarnessRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessRule {
    pub harness: String,
    /// Whether helper chats may run on this harness.
    #[serde(default = "allowed_by_default")]
    pub allowed: bool,
    /// Models helper chats may use here. Empty = any model.
    #[serde(default)]
    pub models: Vec<String>,
    /// The model a helper chat runs on when the agent names none, or names one
    /// it may not use. `None` = the first allowed model, else the harness's
    /// recommended one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Effort every helper chat here runs at. `None` = the agent's choice;
    /// `default` = always the CLI's own configured effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Claude Code only: the model its built-in subagents (the Task tool) use
    /// in every chat on this harness, applied when the CLI starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,
}

fn allowed_by_default() -> bool {
    true
}

impl HarnessRule {
    pub fn new(harness: impl Into<String>) -> Self {
        Self {
            harness: harness.into(),
            allowed: true,
            models: Vec::new(),
            default_model: None,
            effort: None,
            subagent_model: None,
        }
    }

    /// Whether this rule limits anything.
    fn restricts(&self) -> bool {
        !self.allowed
            || !self.models.is_empty()
            || self.default_model.is_some()
            || self.effort.is_some()
            || self.subagent_model.is_some()
    }

    /// The model a helper chat falls back to here, if the user named one.
    fn fallback_model(&self) -> Option<&str> {
        self.default_model
            .as_deref()
            .or_else(|| self.models.first().map(String::as_str))
    }

    /// The allowed-list spelling of `model`, when the list contains it.
    fn allowed_model(&self, model: &str) -> Option<&str> {
        self.models
            .iter()
            .find(|allowed| same_model(allowed, model))
            .map(String::as_str)
    }
}

/// What the agent (or the New Chat settings) asked a helper chat to run on.
#[derive(Debug, Clone, Default)]
pub struct Requested<'a> {
    pub harness: Option<&'a str>,
    /// The agent named the harness (`--harness`), rather than it coming from
    /// the New Chat settings.
    pub explicit_harness: bool,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
}

/// Where a helper chat runs once the limits are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub harness: String,
    /// `None` = the harness's recommended model.
    pub model: Option<String>,
    /// `None` = no override.
    pub effort: Option<String>,
    /// Each substitution made, for the agent that asked.
    pub notes: Vec<String>,
}

impl DelegationPolicy {
    pub fn rule(&self, harness: &str) -> Option<&HarnessRule> {
        self.harnesses.iter().find(|rule| rule.harness == harness)
    }

    /// The rule that applies to `harness` while the limits are on.
    fn active_rule(&self, harness: &str) -> Option<&HarnessRule> {
        self.enabled.then(|| self.rule(harness)).flatten()
    }

    pub fn allows_harness(&self, harness: &str) -> bool {
        self.active_rule(harness).is_none_or(|rule| rule.allowed)
    }

    /// Models helper chats may use on `harness`; empty = any.
    pub fn allowed_models(&self, harness: &str) -> &[String] {
        self.active_rule(harness)
            .map_or(&[], |rule| rule.models.as_slice())
    }

    pub fn allows_model(&self, harness: &str, model: &str) -> bool {
        let models = self.allowed_models(harness);
        models.is_empty() || models.iter().any(|allowed| same_model(allowed, model))
    }

    /// The model a helper chat on `harness` runs on when none is named.
    pub fn default_model(&self, harness: &str) -> Option<&str> {
        self.active_rule(harness)
            .and_then(HarnessRule::fallback_model)
    }

    /// The model Claude Code's own subagents must use, when the user fixed one.
    pub fn claude_subagent_model(&self) -> Option<&str> {
        self.active_rule("claude-code")
            .and_then(|rule| rule.subagent_model.as_deref())
            .filter(|model| !model.trim().is_empty())
    }

    /// Clean a policy the UI sent: known harnesses only, one rule each, no
    /// blank or duplicate models, a default model from the allowed list, and
    /// no rule that limits nothing.
    pub fn normalized(mut self, known: impl Fn(&str) -> bool) -> Result<Self, String> {
        let mut seen: Vec<String> = Vec::new();
        for rule in &mut self.harnesses {
            if !known(&rule.harness) {
                return Err(format!("Unknown harness {}.", rule.harness));
            }
            if seen.contains(&rule.harness) {
                return Err(format!("{} is listed twice.", rule.harness));
            }
            seen.push(rule.harness.clone());
            let mut models: Vec<String> = Vec::new();
            for model in rule.models.drain(..) {
                let model = model.trim().to_string();
                if !model.is_empty() && !models.iter().any(|kept| same_model(kept, &model)) {
                    models.push(model);
                }
            }
            rule.models = models;
            let trim = |value: Option<String>| {
                value
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            };
            rule.default_model = trim(rule.default_model.take());
            rule.effort = trim(rule.effort.take());
            rule.subagent_model = trim(rule.subagent_model.take());
            if let Some(default) = rule.default_model.clone() {
                if !rule.models.is_empty() {
                    rule.default_model = rule.allowed_model(&default).map(str::to_string);
                }
            }
        }
        self.harnesses.retain(HarnessRule::restricts);
        Ok(self)
    }

    /// Apply the limits to one helper chat. `catalog` is the server's harness
    /// catalog (`/api/harnesses`), used for readiness and each model's
    /// reasoning levels; `order` is the harness preference order for when the
    /// requested harness is off. `Err` is a refusal to show the agent.
    pub fn resolve(
        &self,
        requested: &Requested<'_>,
        catalog: &[Value],
        order: &[&str],
    ) -> Result<Decision, String> {
        let mut notes = Vec::new();
        let ready_allowed: Vec<&str> = order
            .iter()
            .copied()
            .filter(|id| self.allows_harness(id) && entry(catalog, id).is_some_and(is_ready))
            .collect();
        let harness = match requested.harness {
            Some(harness) if self.allows_harness(harness) => harness.to_string(),
            Some(harness) if requested.explicit_harness => {
                return Err(refusal(harness, &ready_allowed));
            }
            requested_harness => {
                let Some(first) = ready_allowed.first() else {
                    return Err(
                        "The user's sub-agent limits leave no ready harness, so no chat was \
                         created. Ask the user to allow one in Settings → Harnesses → Sub-agent \
                         limits."
                            .to_string(),
                    );
                };
                if let Some(off) = requested_harness {
                    notes.push(format!(
                        "The New Chat agent {} is off for sub-agent chats; using {}.",
                        display(off),
                        display(first)
                    ));
                }
                first.to_string()
            }
        };
        // A model or effort chosen for another harness means nothing here.
        let same_harness = requested.harness == Some(harness.as_str());
        let rule = self.active_rule(&harness);
        let fallback = rule.and_then(HarnessRule::fallback_model);
        let fallback_text = || match fallback {
            Some(model) => format!("using `{model}`"),
            None => format!("using {}'s recommended model", display(&harness)),
        };

        let asked_model = requested
            .model
            .filter(|_| same_harness)
            .map(str::trim)
            .filter(|model| !model.is_empty());
        let model = match asked_model {
            Some(model) => {
                if let Err(mismatch) =
                    super::harness::model_route::check_model_for_harness(&harness, model)
                {
                    notes.push(format!("{mismatch} Instead {}.", fallback_text()));
                    fallback.map(str::to_string)
                } else if let Some(rule) = rule.filter(|rule| !rule.models.is_empty()) {
                    match rule.allowed_model(model) {
                        Some(allowed) => Some(allowed.to_string()),
                        None => {
                            notes.push(format!(
                                "`{model}` is not among the models the user allows on {} ({}); {}.",
                                display(&harness),
                                code_list(&rule.models),
                                fallback_text()
                            ));
                            fallback.map(str::to_string)
                        }
                    }
                } else {
                    Some(model.to_string())
                }
            }
            None => fallback.map(str::to_string),
        };

        let asked_effort = requested
            .effort
            .filter(|_| same_harness)
            .map(str::trim)
            .filter(|effort| !effort.is_empty());
        let fixed = rule.and_then(|rule| rule.effort.as_deref());
        let effort = match fixed {
            Some(fixed) => {
                if let Some(asked) =
                    asked_effort.filter(|asked| *asked != fixed && *asked != REASONING_DEFAULT_ID)
                {
                    notes.push(format!(
                        "Effort `{asked}` was replaced by `{fixed}`, the effort the user set for \
                         sub-agent chats on {}.",
                        display(&harness)
                    ));
                }
                Some(fixed)
            }
            None => asked_effort,
        };
        let effort = effort.and_then(|effort| {
            if effort == REASONING_DEFAULT_ID {
                return Some(effort.to_string());
            }
            match reasoning_ids(catalog, &harness, model.as_deref()) {
                Some(levels) if !levels.iter().any(|level| level == effort) => {
                    let on = model
                        .as_deref()
                        .map_or_else(|| display(&harness).to_string(), |m| format!("`{m}`"));
                    notes.push(format!(
                        "{on} has no `{effort}` effort; it runs at its default effort."
                    ));
                    None
                }
                _ => Some(effort.to_string()),
            }
        });
        Ok(Decision {
            harness,
            model,
            effort,
            notes,
        })
    }

    /// The playbook bullet telling agents the limits, or `""` while they are off.
    pub fn playbook_bullet(&self, chat_harnesses: &[(&str, &str)]) -> String {
        if !self.enabled {
            return String::new();
        }
        let allowed: Vec<String> = chat_harnesses
            .iter()
            .filter(|(id, _)| self.allows_harness(id))
            .map(|(id, _)| {
                let mut limits = Vec::new();
                let models = self.allowed_models(id);
                if !models.is_empty() {
                    limits.push(format!("models {}", code_list(models)));
                }
                if let Some(model) = self.default_model(id) {
                    limits.push(format!("default `{model}`"));
                }
                if let Some(effort) = self.active_rule(id).and_then(|rule| rule.effort.as_deref()) {
                    limits.push(format!("effort `{effort}`"));
                }
                if limits.is_empty() {
                    format!("`{id}`")
                } else {
                    format!("`{id}` ({})", limits.join(", "))
                }
            })
            .collect();
        let list = if allowed.is_empty() {
            "none — do not create helper chats".to_string()
        } else {
            allowed.join("; ")
        };
        let native = self
            .claude_subagent_model()
            .map(|model| {
                format!(" Claude Code's built-in subagents run on `{model}` whatever model you ask for.")
            })
            .unwrap_or_default();
        format!(
            "\n- Sub-agent limits: **on** — chats you `delegate` to may run only on: {list}. \
             Pick the model and `effort` from this list.{native}"
        )
    }
}

/// The catalog entry for `harness`.
fn entry<'a>(catalog: &'a [Value], harness: &str) -> Option<&'a Value> {
    catalog.iter().find(|entry| entry["id"] == harness)
}

fn is_ready(entry: &Value) -> bool {
    entry["agentReady"] == true
}

/// The reasoning ids `model` (or the harness, for its recommended model)
/// accepts, when the catalog says; `None` when it does not.
fn reasoning_ids(catalog: &[Value], harness: &str, model: Option<&str>) -> Option<Vec<String>> {
    let entry = entry(catalog, harness)?;
    let model = model.or_else(|| entry["recommendedModel"].as_str());
    let levels = model
        .and_then(|model| {
            entry["models"]
                .as_array()?
                .iter()
                .find(|item| item["id"].as_str().is_some_and(|id| same_model(id, model)))
        })
        .and_then(|item| item["reasoningLevels"].as_array())
        .or_else(|| entry.pointer("/options/reasoningLevels")?.as_array())?;
    // An empty list is known: the model has no effort control at all.
    Some(
        levels
            .iter()
            .filter_map(|level| level["id"].as_str().map(str::to_string))
            .collect(),
    )
}

/// Model ids are compared case-insensitively, with or without OpenCode's
/// `provider/` prefix, and through their canonical alias resolution
/// (`claude-opus` = `opus`).
pub fn same_model(left: &str, right: &str) -> bool {
    let bare = |id: &str| {
        let id = id.trim().to_ascii_lowercase();
        match id.split_once('/') {
            Some((_, rest)) => rest.to_string(),
            None => id,
        }
    };
    let (left, right) = (bare(left), bare(right));
    if left == right {
        return true;
    }
    let canonical = |id: &str| {
        super::harness::model_route::resolve_model(id)
            .ok()
            .map(|resolved| bare(&resolved.model_id))
    };
    matches!((canonical(&left), canonical(&right)), (Some(a), Some(b)) if a == b)
}

fn display(harness: &str) -> &str {
    super::harness::model_route::harness_display_name(harness)
}

fn code_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn refusal(harness: &str, ready_allowed: &[&str]) -> String {
    let name = display(harness);
    if ready_allowed.is_empty() {
        format!(
            "The user turned {name} off for sub-agent chats and no allowed harness is ready, so \
             no chat was created. Ask the user to allow one in Settings → Harnesses → Sub-agent \
             limits."
        )
    } else {
        let list: Vec<String> = ready_allowed.iter().map(|id| format!("`{id}`")).collect();
        format!(
            "The user turned {name} off for sub-agent chats, so no chat was created. Use \
             `--harness` with one of: {}.",
            list.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> Vec<Value> {
        vec![
            json!({"id": "claude-code", "agentReady": true, "recommendedModel": "opus",
            "models": [
                {"id": "opus", "reasoningLevels": [{"id": "default"}, {"id": "low"}, {"id": "high"}]},
                {"id": "haiku", "reasoningLevels": []},
            ]}),
            json!({"id": "codex", "agentReady": true, "recommendedModel": "gpt-6.1-sol",
                "models": [{"id": "gpt-6.1-sol", "reasoningLevels": [{"id": "medium"}, {"id": "xhigh"}]}]}),
            json!({"id": "cursor", "agentReady": false, "models": []}),
        ]
    }

    const ORDER: &[&str] = &["claude-code", "codex", "cursor"];

    fn policy(rules: Vec<HarnessRule>) -> DelegationPolicy {
        DelegationPolicy {
            enabled: true,
            harnesses: rules,
        }
    }

    #[test]
    fn off_changes_nothing() {
        let mut rule = HarnessRule::new("codex");
        rule.allowed = false;
        let policy = DelegationPolicy {
            enabled: false,
            harnesses: vec![rule],
        };
        let decision = policy
            .resolve(
                &Requested {
                    harness: Some("codex"),
                    explicit_harness: true,
                    model: Some("whatever"),
                    effort: Some("xhigh"),
                },
                &catalog(),
                ORDER,
            )
            .unwrap();
        assert_eq!(decision.harness, "codex");
        assert_eq!(decision.model.as_deref(), Some("whatever"));
        assert_eq!(decision.effort.as_deref(), Some("xhigh"));
        assert!(decision.notes.is_empty());
        assert!(policy.claude_subagent_model().is_none());
    }

    #[test]
    fn a_harness_turned_off_is_refused_when_named_and_skipped_when_implicit() {
        let mut rule = HarnessRule::new("claude-code");
        rule.allowed = false;
        let policy = policy(vec![rule]);
        let refusal = policy
            .resolve(
                &Requested {
                    harness: Some("claude-code"),
                    explicit_harness: true,
                    ..Default::default()
                },
                &catalog(),
                ORDER,
            )
            .unwrap_err();
        assert!(refusal.contains("`codex`"), "{refusal}");

        let decision = policy
            .resolve(
                &Requested {
                    harness: Some("claude-code"),
                    model: Some("opus"),
                    effort: Some("high"),
                    ..Default::default()
                },
                &catalog(),
                ORDER,
            )
            .unwrap();
        assert_eq!(decision.harness, "codex");
        // The New Chat model and effort belonged to the harness that was off.
        assert_eq!(decision.model, None);
        assert_eq!(decision.effort, None);
        assert_eq!(decision.notes.len(), 1);
    }

    #[test]
    fn no_ready_allowed_harness_is_a_refusal() {
        let rules = ["claude-code", "codex"]
            .into_iter()
            .map(|id| {
                let mut rule = HarnessRule::new(id);
                rule.allowed = false;
                rule
            })
            .collect();
        let policy = policy(rules);
        assert!(policy
            .resolve(&Requested::default(), &catalog(), ORDER)
            .is_err());
    }

    #[test]
    fn a_model_outside_the_list_is_replaced_by_the_default() {
        let mut rule = HarnessRule::new("claude-code");
        rule.models = vec!["sonnet".into(), "opus".into()];
        rule.default_model = Some("sonnet".into());
        let policy = policy(vec![rule]);
        let ask = |model| Requested {
            harness: Some("claude-code"),
            explicit_harness: true,
            model,
            effort: None,
        };
        let decision = policy
            .resolve(&ask(Some("haiku")), &catalog(), ORDER)
            .unwrap();
        assert_eq!(decision.model.as_deref(), Some("sonnet"));
        assert!(
            decision.notes[0].contains("`haiku`"),
            "{:?}",
            decision.notes
        );

        // Allowed, spelled another way: kept, in the list's spelling.
        let decision = policy
            .resolve(&ask(Some("Claude-Opus")), &catalog(), ORDER)
            .unwrap();
        assert_eq!(decision.model.as_deref(), Some("opus"));
        assert!(decision.notes.is_empty());

        let decision = policy.resolve(&ask(None), &catalog(), ORDER).unwrap();
        assert_eq!(decision.model.as_deref(), Some("sonnet"));
        assert!(decision.notes.is_empty());
    }

    #[test]
    fn another_vendors_model_is_replaced_even_without_a_list() {
        let policy = policy(Vec::new());
        let decision = policy
            .resolve(
                &Requested {
                    harness: Some("claude-code"),
                    explicit_harness: true,
                    model: Some("gpt-6.1-sol"),
                    effort: None,
                },
                &catalog(),
                ORDER,
            )
            .unwrap();
        assert_eq!(decision.model, None);
        assert_eq!(decision.notes.len(), 1, "{:?}", decision.notes);
    }

    #[test]
    fn a_fixed_effort_wins_and_unsupported_efforts_fall_back() {
        let mut rule = HarnessRule::new("claude-code");
        rule.effort = Some("low".into());
        let policy = policy(vec![rule]);
        let decision = policy
            .resolve(
                &Requested {
                    harness: Some("claude-code"),
                    explicit_harness: true,
                    model: Some("opus"),
                    effort: Some("high"),
                },
                &catalog(),
                ORDER,
            )
            .unwrap();
        assert_eq!(decision.effort.as_deref(), Some("low"));
        assert_eq!(decision.notes.len(), 1);

        // Haiku has no effort control: the fixed effort is dropped.
        let decision = policy
            .resolve(
                &Requested {
                    harness: Some("claude-code"),
                    explicit_harness: true,
                    model: Some("haiku"),
                    effort: None,
                },
                &catalog(),
                ORDER,
            )
            .unwrap();
        assert_eq!(decision.effort, None);

        // The agent's own pick is checked against the model too.
        let decision = DelegationPolicy {
            enabled: true,
            harnesses: Vec::new(),
        }
        .resolve(
            &Requested {
                harness: Some("codex"),
                explicit_harness: true,
                model: None,
                effort: Some("ultra"),
            },
            &catalog(),
            ORDER,
        )
        .unwrap();
        assert_eq!(decision.effort, None);
        assert_eq!(decision.notes.len(), 1);
    }

    #[test]
    fn normalizing_drops_empty_rules_and_a_default_outside_the_list() {
        let mut rule = HarnessRule::new("codex");
        rule.models = vec![" gpt-6.1-sol ".into(), "".into(), "GPT-6.1-sol".into()];
        rule.default_model = Some("gpt-5".into());
        let policy = DelegationPolicy {
            enabled: true,
            harnesses: vec![rule, HarnessRule::new("claude-code")],
        }
        .normalized(|id| ORDER.contains(&id))
        .unwrap();
        assert_eq!(policy.harnesses.len(), 1);
        assert_eq!(policy.harnesses[0].models, ["gpt-6.1-sol"]);
        assert_eq!(policy.harnesses[0].default_model, None);

        assert!(DelegationPolicy {
            enabled: true,
            harnesses: vec![HarnessRule::new("nope")],
        }
        .normalized(|id| ORDER.contains(&id))
        .is_err());
    }

    #[test]
    fn the_playbook_lists_only_allowed_harnesses_with_their_limits() {
        let mut off = HarnessRule::new("codex");
        off.allowed = false;
        let mut claude = HarnessRule::new("claude-code");
        claude.models = vec!["opus".into()];
        claude.effort = Some("high".into());
        claude.subagent_model = Some("sonnet".into());
        let policy = policy(vec![off, claude]);
        let bullet = policy.playbook_bullet(&[("claude-code", "Claude Code"), ("codex", "Codex")]);
        assert!(
            bullet.contains("`claude-code` (models `opus`, default `opus`, effort `high`)"),
            "{bullet}"
        );
        assert!(!bullet.contains("`codex`"), "{bullet}");
        assert!(
            bullet.contains("built-in subagents run on `sonnet`"),
            "{bullet}"
        );
        assert_eq!(DelegationPolicy::default().playbook_bullet(&[]), "");
    }

    #[test]
    fn stored_rules_default_to_allowed() {
        let rule: HarnessRule = serde_json::from_value(json!({"harness": "codex"})).unwrap();
        assert!(rule.allowed);
    }
}
