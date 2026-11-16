//! Steers an agent's shell call that launches experiment compute around orx
//! back to `orx exp run`.
//!
//! The playbook and `orx-compute` already say every experiment runs through
//! `orx exp run`; a run started any other way has no recorded commit and its
//! result never reaches the chat. Claude Code's `PreToolUse` hook (`orx
//! invocation-gate`) asks [`bypass_reason`] about each Bash call and denies the
//! ones that clearly bypass orx, with a reason naming the `orx` command to use.
//!
//! Only clear launches are denied: a scheduler or provider job submission, or
//! the project's own run command typed as a shell command. Everything else,
//! including `orx compute configure slurm --sbatch-arg=…` and ordinary scripts,
//! passes untouched.

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

/// Why `command` must go through `orx exp run` instead, or `None` when it may run.
/// `run_commands` are the project's fixed run commands (project default and per
/// experiment).
pub fn bypass_reason(command: &str, run_commands: &[String]) -> Option<String> {
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
    }
    None
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
/// newlines. Not quote-aware; a separator inside quotes only splits a segment
/// that then matches nothing.
fn segments(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(['\n', ';', '|', '&'])
        .map(str::trim)
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
            let reason = bypass_reason(command, &[]).unwrap_or_else(|| panic!("{command}"));
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
            assert!(bypass_reason(command, &runs).is_some(), "{command}");
        }
    }

    #[test]
    fn run_command_setup_and_redirects_do_not_hide_it() {
        let runs = run("cd src && python train.py > out.log 2>&1");
        assert!(bypass_reason("python train.py --epochs 1", &runs).is_some());
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
            assert_eq!(bypass_reason(command, &runs), None, "{command}");
        }
    }

    #[test]
    fn a_bare_interpreter_run_command_matches_nothing() {
        assert_eq!(
            bypass_reason("python scripts/plot.py", &run("python")),
            None
        );
        assert_eq!(bypass_reason("make test", &run("make")), None);
    }
}
