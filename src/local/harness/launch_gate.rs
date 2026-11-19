//! Steers an agent's shell call that launches experiment compute around orx
//! back to `orx exp run`.
//!
//! The playbook and `orx-compute` already say every experiment runs through
//! `orx exp run`; a run started any other way has no recorded commit and its
//! result never reaches the chat. Claude Code's `PreToolUse` hook (`orx
//! launch-gate`, [`hook`]) and Antigravity's gate ask [`bypass_reason`] about each
//! shell call and deny the ones that clearly bypass orx, with a reason naming the
//! `orx` command to use.
//!
//! Only clear launches are denied: a scheduler or provider job submission, the
//! project's own run command typed as a shell command, or a local script that
//! submits such a job (an agent's home-made launcher). Everything else,
//! including `orx compute configure slurm --sbatch-arg=…` and ordinary scripts,
//! passes untouched.

use std::path::{Path, PathBuf};

use std::io::Read;

use serde_json::{json, Value};

use crate::error::Result;
use crate::store::Store;

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
        eprintln!("orx launch-gate: cannot read the project's run commands: {error}");
        Vec::new()
    })
}

/// Claude Code's `PreToolUse` hook body: a Bash call that bypasses orx is denied.
pub(crate) fn hook() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let payload: Value = serde_json::from_str(&input)?;
    if let Some(reason) = deny_reason(&payload) {
        println!(
            "{}",
            json!({"hookSpecificOutput":{
                "hookEventName":"PreToolUse",
                "permissionDecision":"deny",
                "permissionDecisionReason":reason,
            }})
        );
    }
    Ok(())
}

fn deny_reason(payload: &Value) -> Option<String> {
    if payload.get("tool_name").and_then(Value::as_str) != Some("Bash") {
        return None;
    }
    let command = payload.pointer("/tool_input/command")?.as_str()?;
    let cwd = payload.get("cwd").and_then(Value::as_str).map(Path::new);
    let commands = match Store::open() {
        Ok(store) => run_commands(&store),
        Err(error) => {
            eprintln!("orx launch-gate: cannot open the store for run commands: {error}");
            Vec::new()
        }
    };
    bypass_reason(command, &commands, cwd)
}

/// Programs whose only job is to submit work to a cluster scheduler.
const SCHEDULERS: &[&str] = &["sbatch", "srun", "salloc", "qsub", "bsub"];

/// Provider CLI verbs that start a remote job: program, then its verb words.
const PROVIDER_LAUNCHES: &[&[&str]] = &[
    &["kaggle", "kernels", "push"],
    &["modal", "run"],
    &["ray", "job", "submit"],
    &["hf", "jobs", "run"],
];

/// Wrappers that run the rest of the line as the real command.
const WRAPPERS: &[&str] = &[
    "nohup",
    "time",
    "exec",
    "setsid",
    "nice",
    "stdbuf",
    "caffeinate",
];

/// A run command that is only one of these says nothing about the experiment.
const BARE_PROGRAMS: &[&str] = &["python", "python3", "bash", "sh", "uv", "node", "make"];

/// Scripts larger than this are not read; a launcher is a few kilobytes.
const SCRIPT_LIMIT: u64 = 256 * 1024;

/// Why `command` must go through `orx exp run` instead, or `None` when it may run.
/// `run_commands` are the project's fixed run commands (project default and per
/// experiment); `cwd` resolves a relative script path.
pub fn bypass_reason(command: &str, run_commands: &[String], cwd: Option<&Path>) -> Option<String> {
    let run_prefixes: Vec<Vec<String>> = run_commands
        .iter()
        .filter_map(|run| launch_tokens(run))
        .collect();
    let command = merge_redirects(command);
    for segment in segments(&command) {
        let tokens = strip_wrappers(segment.split_whitespace().collect());
        let Some(&first) = tokens.first() else {
            continue;
        };
        let program = first.rsplit('/').next().unwrap_or(first);
        if SCHEDULERS.contains(&program) {
            return Some(format!(
                "Blocked: `{program}` submits compute outside orx, so the run has no recorded \
                 commit and its result never reaches this chat. Configure the cluster as a \
                 backend (`orx compute configure slurm --help`) and launch with `orx exp run \
                 <experimentId>`. Load the orx-compute skill first."
            ));
        }
        if let Some(launch) = PROVIDER_LAUNCHES.iter().find(|launch| {
            launch[0] == program
                && tokens.len() >= launch.len()
                && tokens[1..launch.len()] == launch[1..]
        }) {
            return Some(format!(
                "Blocked: `{}` starts a job outside orx, so the run has no recorded commit and \
                 its result never reaches this chat. Launch it with `orx exp run \
                 <experimentId>` on the configured backend (`orx compute status`). Load the \
                 orx-compute skill first.",
                launch.join(" ")
            ));
        }
        // Quotes fall away so a run command inside `ssh host '…'` still matches.
        let words = unquoted(&tokens);
        let typed = run_prefixes.iter().any(|run| {
            let run: Vec<&str> = run.iter().map(String::as_str).collect();
            words.starts_with(&run) || (program == "ssh" && contains_tokens(&words, &run))
        });
        if typed {
            return Some(
                "Blocked: this runs the project's experiment command directly, outside orx, so \
                 the result is not recorded against a commit and never reaches this chat. \
                 Commit the change on the experiment's branch and launch it with `orx exp run \
                 <experimentId>`; the result arrives here as a message. Load the orx-compute \
                 skill first."
                    .into(),
            );
        }
        if let Some((script, launch)) = script_path(&tokens, cwd)
            .and_then(|script| script_launch(&script).map(|launch| (script, launch)))
        {
            return Some(format!(
                "Blocked: `{}` submits compute outside orx (`{launch}`), so the run has no \
                 recorded commit, never appears in Experiments or Results, and its result never \
                 reaches this chat. Launch it with `orx exp run <experimentId>` on the configured \
                 backend (`orx compute status`). If `orx exp run` itself fails, tell the user \
                 what failed instead of building a launcher. Load the orx-compute skill first.",
                script.display()
            ));
        }
    }
    None
}

/// The local script file a simple command runs: the first operand of a shell
/// (`bash x.sh`, `source x.sh`) or the program itself when it is a path.
fn script_path(tokens: &[&str], cwd: Option<&Path>) -> Option<PathBuf> {
    let first = *tokens.first()?;
    let name = first.rsplit('/').next().unwrap_or(first);
    let candidate = if matches!(name, "bash" | "sh" | "zsh" | "dash" | "source" | ".") {
        let operand = tokens[1..].iter().find(|t| !t.starts_with('-'))?;
        if tokens[1..]
            .iter()
            .take_while(|t| t.starts_with('-'))
            .any(|t| t.contains('c'))
        {
            return None;
        }
        operand
    } else if first.contains('/') {
        first
    } else {
        return None;
    };
    let candidate = candidate.trim_matches(['\'', '"']);
    if candidate.contains('$') {
        return None;
    }
    let path = match candidate.strip_prefix("~/") {
        Some(rest) => crate::config::home_dir()?.join(rest),
        None => PathBuf::from(candidate),
    };
    let path = if path.is_relative() {
        cwd?.join(path)
    } else {
        path
    };
    let metadata = std::fs::metadata(&path).ok()?;
    (metadata.is_file() && metadata.len() <= SCRIPT_LIMIT).then_some(path)
}

/// The scheduler or provider launch a script makes, judged by its words rather
/// than its lines, since launchers wrap the CLI in variables and functions.
fn script_launch(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let words: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .flat_map(|line| {
            line.split(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '"' | '\'' | '=' | '(' | ')' | ';' | '|' | '&' | '`' | '{' | '}'
                    )
            })
        })
        .filter(|word| !word.is_empty())
        .collect();
    let program = |word: &str| word.rsplit('/').next().unwrap_or(word).to_string();
    if let Some(scheduler) = words
        .iter()
        .map(|word| program(word))
        .find(|word| SCHEDULERS.contains(&word.as_str()))
    {
        return Some(scheduler);
    }
    PROVIDER_LAUNCHES
        .iter()
        .find(|launch| {
            words.iter().any(|word| program(word) == launch[0])
                && contains_tokens(&words, &launch[1..])
        })
        .map(|launch| launch.join(" "))
}

/// The tokens that identify a run command: its last shell segment (the part
/// after any `cd … &&` setup), wrappers and redirections removed. `None` when
/// that is only a bare interpreter, which would match unrelated commands.
fn launch_tokens(run: &str) -> Option<Vec<String>> {
    let run = merge_redirects(run);
    let segment = segments(&run).last()?;
    let tokens: Vec<String> = strip_wrappers(segment.split_whitespace().collect())
        .into_iter()
        .take_while(|token| !is_redirect(token))
        .map(str::to_string)
        .collect();
    match tokens.as_slice() {
        [] => None,
        [only] if BARE_PROGRAMS.contains(&only.as_str()) => None,
        _ => Some(tokens),
    }
}

/// `2>&1` and `&>` redirect rather than background, so they must not split a segment.
fn merge_redirects(command: &str) -> String {
    command.replace(">&", "> ").replace("&>", "> ")
}

/// The simple commands of a shell line: split on `;`, `&&`, `||`, `|`, `&` and
/// newlines, without subshell parentheses. Not quote-aware; a separator inside quotes only splits a segment
/// that then matches nothing.
fn segments(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(['\n', ';', '|', '&'])
        // A subshell's parentheses or a group's braces wrap the command, not name it.
        .map(|segment| {
            segment
                .trim()
                .trim_start_matches(['(', '{', ' '])
                .trim_end_matches([')', '}', ' '])
        })
        .filter(|segment| !segment.is_empty())
}

/// Drop leading env assignments, `env`, `sudo`, `timeout <duration>` and
/// [`WRAPPERS`], leaving the program the line actually runs.
fn strip_wrappers(mut tokens: Vec<&str>) -> Vec<&str> {
    loop {
        let Some(&first) = tokens.first() else {
            return tokens;
        };
        let name = first.rsplit('/').next().unwrap_or(first);
        let skip = if is_assignment(first) || WRAPPERS.contains(&name) {
            1
        } else if matches!(name, "env" | "sudo") {
            1 + tokens[1..]
                .iter()
                .take_while(|t| t.starts_with('-') || is_assignment(t))
                .count()
        } else if name == "timeout" {
            let flags = tokens[1..]
                .iter()
                .take_while(|t| t.starts_with('-'))
                .count();
            (2 + flags).min(tokens.len())
        } else {
            return tokens;
        };
        tokens.drain(..skip);
    }
}

fn is_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

fn is_redirect(token: &str) -> bool {
    token.starts_with(['>', '<']) || token.starts_with("2>") || token.starts_with("1>")
}

fn unquoted<'a>(tokens: &[&'a str]) -> Vec<&'a str> {
    tokens
        .iter()
        .map(|t| t.trim_matches(['\'', '"']))
        .filter(|t| !t.is_empty())
        .collect()
}

fn contains_tokens(haystack: &[&str], needle: &[&str]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_denies_only_bash_calls_that_bypass_orx() {
        let bash = |command: &str| json!({"tool_name": "Bash", "tool_input": {"command": command}});
        assert!(deny_reason(&bash("sbatch train.sh"))
            .is_some_and(|reason| reason.contains("orx exp run")));
        assert!(deny_reason(&bash("git status")).is_none());
        let edit = json!({"tool_name": "Edit", "tool_input": {"command": "sbatch train.sh"}});
        assert!(deny_reason(&edit).is_none());
    }

    fn scratch() -> PathBuf {
        let dir = crate::config::test_root().join(format!("launch-gate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(cmd: &str) -> Vec<String> {
        vec![cmd.to_string()]
    }

    #[test]
    fn schedulers_and_provider_launches_are_denied() {
        for command in [
            "sbatch job.sh",
            "cd /scratch && /usr/bin/sbatch --gres=gpu:1 run.sbatch",
            "srun -p gpu python train.py",
            "qsub job.pbs",
            "kaggle kernels push -p kernel/",
            "nohup modal run train.py &",
            "ray job submit -- python train.py",
            "hf jobs run pytorch/pytorch python train.py",
        ] {
            let reason = bypass_reason(command, &[], None).unwrap_or_else(|| panic!("{command}"));
            assert!(reason.contains("orx exp run"), "{command}: {reason}");
        }
    }

    #[test]
    fn the_run_command_typed_directly_is_denied() {
        let runs = run("uv run --locked python train.py --config configs/base.yaml");
        for command in [
            "uv run --locked python train.py --config configs/base.yaml",
            "cd src && uv run --locked python train.py --config configs/base.yaml --steps 10",
            "CUDA_VISIBLE_DEVICES=0 nohup uv run --locked python train.py --config configs/base.yaml > log.txt 2>&1 &",
            "timeout 60 uv run --locked python train.py --config configs/base.yaml",
            "ssh gpu-box 'cd repo && uv run --locked python train.py --config configs/base.yaml'",
        ] {
            assert!(bypass_reason(command, &runs, None).is_some(), "{command}");
        }
    }

    #[test]
    fn run_command_setup_and_redirects_do_not_hide_it() {
        let runs = run("cd src && python train.py > out.log 2>&1");
        assert!(bypass_reason("python train.py --epochs 1", &runs, None).is_some());
    }

    #[test]
    fn ordinary_work_passes() {
        let runs = run("python train.py");
        for command in [
            "orx exp run exp_123",
            "orx compute configure slurm --sbatch-arg='--rsc p=1:g=1'",
            "python -c 'import torch; print(torch.__version__)'",
            "python scripts/plot.py",
            "uv run --isolated --no-project --with pymupdf python extract.py",
            "git commit -m 'sbatch notes'",
            "grep -rn srun docs/",
            "ssh gpu-box nvidia-smi",
            "kaggle kernels status me/kernel",
            "modal token new",
            "cat train.py",
        ] {
            assert_eq!(bypass_reason(command, &runs, None), None, "{command}");
        }
    }

    #[test]
    fn a_home_made_launcher_script_is_denied() {
        let dir = scratch();
        std::fs::create_dir(dir.join("infra")).unwrap();
        let launcher = dir.join("infra/kaggle_run.sh");
        std::fs::write(
            &launcher,
            "#!/usr/bin/env bash\n# runs bash run.sh as a Kaggle kernel\n\
             K=$HOME/.local/share/uv/tools/kaggle/bin/kaggle\n\
             kg() { HOME=$H $K \"$@\" 2>&1; }\n\
             retry 6 \"successfully pushed\" kernels push -p \"$W/k\"\n",
        )
        .unwrap();
        let slurm = dir.join("submit.sh");
        std::fs::write(
            &slurm,
            "set -e\nJOB=$(/usr/bin/sbatch --parsable job.sbatch)\n",
        )
        .unwrap();
        let absolute = launcher.display().to_string();
        for command in [
            format!("{absolute} acct r1 0ea1d6c"),
            format!(
                "(EXTRA_ENV=\"A=1\" setsid nohup bash {absolute} acct r5 9eb2ebc > out 2>&1 &)"
            ),
            "bash infra/kaggle_run.sh acct r1 abc".to_string(),
            "./infra/kaggle_run.sh acct r1 abc".to_string(),
            "sh submit.sh".to_string(),
        ] {
            let reason =
                bypass_reason(&command, &[], Some(&dir)).unwrap_or_else(|| panic!("{command}"));
            assert!(reason.contains("orx exp run"), "{command}: {reason}");
        }
    }

    #[test]
    fn ordinary_scripts_pass() {
        let dir = scratch();
        std::fs::write(
            dir.join("plot.sh"),
            "# not sbatch, not kaggle kernels push\npython plot.py\nkaggle kernels status me/k\n\
             orx compute configure slurm --sbatch-arg=--gres=gpu:1\n",
        )
        .unwrap();
        for command in [
            "bash plot.sh",
            "./plot.sh",
            "bash missing.sh",
            "bash $R/launcher.sh",
        ] {
            assert_eq!(bypass_reason(command, &[], Some(&dir)), None, "{command}");
        }
    }

    #[test]
    fn a_bare_interpreter_run_command_matches_nothing() {
        assert_eq!(
            bypass_reason("python scripts/plot.py", &run("python"), None),
            None
        );
        assert_eq!(bypass_reason("make test", &run("make"), None), None);
    }
}
