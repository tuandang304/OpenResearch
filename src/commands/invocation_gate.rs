use std::io::Read;

use serde_json::{json, Value};

use crate::error::{anyhow, Result};
use crate::store::{InvocationIdentity, Store};

/// `command` run with the native invoking identity exported to `orx exp run`.
pub(crate) fn with_invocation_context(
    command: &str,
    identity: &InvocationIdentity,
) -> Result<String> {
    identity.validate()?;
    Ok(format!(
        "export ORX_INVOCATION_CONTEXT={}; {command}",
        crate::jobs::ssh::sh_quote(&serde_json::to_string(identity)?)
    ))
}

fn rewrite(payload: &Value, identity: &InvocationIdentity) -> Result<Value> {
    let mut input = payload
        .get("tool_input")
        .cloned()
        .ok_or_else(|| anyhow!("Missing native tool input"))?;
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing native shell command"))?;
    input["command"] = json!(with_invocation_context(command, identity)?);
    Ok(json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":input}}))
}

fn deny(reason: &str) -> Value {
    json!({"hookSpecificOutput":{
        "hookEventName":"PreToolUse",
        "permissionDecision":"deny",
        "permissionDecisionReason":reason,
    }})
}

/// The fixed run commands of the chat's project. Best effort: without them the
/// gate still catches scheduler and provider launches.
pub(crate) fn run_commands(store: &Store) -> Vec<String> {
    let lookup = || -> Result<Vec<String>> {
        let Some(session) = crate::local::chat::launching_chat_session() else {
            return Ok(Vec::new());
        };
        let Some(session) = store.get_chat_session(&session)? else {
            return Ok(Vec::new());
        };
        let mut commands: Vec<String> = store
            .list_experiments_by_project(&session.project_id)?
            .into_iter()
            .map(|experiment| experiment.run_command)
            .collect();
        if let Some(project) = store.get_local_project(&session.project_id)? {
            commands.extend(project.run_command);
        }
        commands.retain(|command| !command.trim().is_empty());
        commands.sort();
        commands.dedup();
        Ok(commands)
    };
    lookup().unwrap_or_else(|error| {
        eprintln!("orx invocation-gate: cannot read the project's run commands: {error}");
        Vec::new()
    })
}

pub async fn run() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let payload: Value = serde_json::from_str(&input)?;
    if payload.get("tool_name").and_then(Value::as_str) != Some("Bash") {
        return Ok(());
    }
    let store = Store::open()?;
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(std::path::Path::new);
    if let Some(reason) = payload
        .pointer("/tool_input/command")
        .and_then(Value::as_str)
        .and_then(|command| {
            crate::local::harness::launch_gate::bypass_reason(command, &run_commands(&store), cwd)
        })
    {
        println!("{}", deny(&reason));
        return Ok(());
    }
    let call_id = payload
        .get("tool_use_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing native tool-use identity"))?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(identity) = store.native_invocation_identity("claude-code", call_id)? {
            println!("{}", rewrite(&payload, &identity)?);
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "Native tool model was not captured for this invocation"
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_preserves_arguments_and_permission_decisions() {
        let identity = InvocationIdentity {
            harness: "claude-code".into(),
            model: "claude-opus-5-5".into(),
            provider: None,
        };
        let payload = json!({"tool_input":{"command":"printf 'ok' > output &", "timeout":1000}});
        let response = rewrite(&payload, &identity).unwrap();
        assert!(response["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none());
        assert_eq!(
            response["hookSpecificOutput"]["updatedInput"]["timeout"],
            1000
        );
        let command = response["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        assert!(command.ends_with("; printf 'ok' > output &"));
        assert!(command.contains("claude-opus-5-5"));
    }
}
