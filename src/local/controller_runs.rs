//! Launch intents for the backends a controller on this machine drives: Google
//! Colab and Kaggle through their CLIs, and Google Cloud through `orx gcp-run`.
//! Owned by the tuandang304 fork; see docs/fork-maintenance.md.
//!
//! Each one is a local engine run (`engine::adapters::local`) whose `run.sh`
//! writes the controller's files into the run dir and then executes it, so the
//! stored intent alone can start the run again. The controller's output is the
//! run's log and its exit code the run's.

use std::path::Path;

use serde_json::{json, Value};

use crate::compute::SourceSnapshot;
use crate::error::{anyhow, Result};
use crate::jobs::ssh::sh_quote;
use crate::store::{LaunchSpec, Store, StoredRun};

/// A `run.sh` that writes `files` (paths relative to the run dir) and runs
/// `then`. `printf` is a bash builtin, so no file is too long for argv.
fn materialize(files: &[(String, String)], then: &str) -> String {
    let mut script = String::from("set -e\numask 077\n");
    for (path, contents) in files {
        if let Some(parent) = Path::new(path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            script.push_str(&format!(
                "mkdir -p {}\n",
                sh_quote(&parent.to_string_lossy())
            ));
        }
        script.push_str(&format!(
            "printf '%s' {} > {}\n",
            sh_quote(contents),
            sh_quote(path)
        ));
    }
    script.push_str("set +e\n");
    script.push_str(then);
    script.push('\n');
    script
}

fn spec(kind: &str, target: Option<String>, run_sh: String, extra: Value) -> LaunchSpec {
    let mut options = json!({ "run_sh": run_sh });
    if let (Some(options), Value::Object(extra)) = (options.as_object_mut(), extra) {
        options.extend(extra.into_iter().filter(|(_, value)| !value.is_null()));
    }
    LaunchSpec {
        kind: kind.into(),
        target,
        options,
    }
}

fn timeout(args: &crate::ExpRunArgs, default: u64) -> Result<u64> {
    match args.timeout.as_deref() {
        Some(timeout) => crate::jobs::huggingface::parse_timeout(timeout),
        None => Ok(default),
    }
}

/// Names of the variables the run gets, whose values the controller reads from
/// its own environment at launch.
fn forwarded_env_keys(run_id: &str) -> Vec<String> {
    let mut keys: Vec<String> = crate::config::launch_env(run_id).into_keys().collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Synced secrets and the Hugging Face token: what a Kaggle kernel cannot be
/// given, since its source is stored on Kaggle.
fn synced_secret_keys() -> Vec<String> {
    let mut keys: Vec<String> = crate::config::list_synced_env()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    if crate::jobs::huggingface::resolve_token().is_ok() {
        keys.push("HF_TOKEN".into());
    }
    keys.sort();
    keys.dedup();
    keys
}

pub async fn colab_launch_spec(
    args: &crate::ExpRunArgs,
    source: &SourceSnapshot,
    run_id: &str,
    command: &str,
) -> Result<LaunchSpec> {
    if args.image.is_some() {
        return Err(anyhow!(
            "--image doesn't apply to --backend colab — the runtime uses Colab's own image."
        ));
    }
    let flavor = crate::jobs::colab::parse_flavor(args.flavor.as_deref())?;
    let cli = crate::jobs::colab::preflight().await?;
    let timeout_secs = timeout(args, crate::jobs::colab::DEFAULT_TIMEOUT_SECS)?;
    // Recorded with the account it ran on, so the run says which one paid.
    let account = crate::jobs::colab_accounts::resolve_account(&flavor).await?;
    let flavor = flavor.on_account(&account);
    let session = crate::jobs::colab::session_name(run_id);
    let controller = crate::jobs::colab::controller_script(&crate::jobs::colab::ControllerSpec {
        cli: &cli,
        home: &crate::jobs::colab_accounts::account_home(&account),
        session: &session,
        flavor: &flavor,
        archive: &source.path,
        timeout_secs,
        env_keys: forwarded_env_keys(run_id),
    });
    let files = [
        (crate::jobs::colab::CONTROLLER_FILE.to_string(), controller),
        (
            crate::jobs::colab::DRIVER_FILE.to_string(),
            crate::jobs::colab::driver_source(command),
        ),
    ];
    let run_sh = materialize(
        &files,
        &format!(
            "exec bash {}",
            sh_quote(crate::jobs::colab::CONTROLLER_FILE)
        ),
    );
    Ok(spec(
        "colab_job",
        Some(session),
        run_sh,
        json!({ "flavor": flavor.id(), "timeout_seconds": timeout_secs }),
    ))
}

pub async fn kaggle_launch_spec(
    args: &crate::ExpRunArgs,
    source: &SourceSnapshot,
    run_id: &str,
    command: &str,
) -> Result<LaunchSpec> {
    if args.image.is_some() {
        return Err(anyhow!(
            "--image doesn't apply to --backend kaggle — the kernel uses Kaggle's own image."
        ));
    }
    let flavor = crate::jobs::kaggle::parse_flavor(args.flavor.as_deref())?;
    let cli = crate::jobs::kaggle::preflight().await?;
    let timeout_secs = timeout(
        args,
        crate::jobs::kaggle::DEFAULT_TIMEOUT_SECS.min(flavor.timeout_cap()),
    )?;
    if timeout_secs > flavor.timeout_cap() {
        return Err(anyhow!(
            "Kaggle stops a {} session after {} hours; pass a shorter --timeout.",
            flavor.accelerator.kind.to_uppercase(),
            flavor.timeout_cap() / 3600
        ));
    }
    let username =
        crate::jobs::kaggle::resolve_account(&flavor.account, flavor.accelerator.kind).await?;
    let files = crate::jobs::kaggle::run_files(&crate::jobs::kaggle::RunSpec {
        cli: &cli,
        username: &username,
        run_id,
        flavor: &flavor,
        run_command: command,
        timeout_secs,
        skipped_env: synced_secret_keys(),
    })?;
    let archive = sh_quote(&crate::local::bash::bash_path(&source.path));
    let copy = sh_quote(&crate::jobs::kaggle::archive_path());
    let run_sh = materialize(
        &files,
        &format!(
            "ln -f {archive} {copy} 2>/dev/null || cp {archive} {copy} || {{ echo '[orx] Could not stage the source snapshot.'; exit 1; }}\nexec bash {}",
            sh_quote(crate::jobs::kaggle::CONTROLLER_FILE)
        ),
    );
    let slug = crate::jobs::kaggle::run_slug(run_id);
    Ok(spec(
        "kaggle_job",
        Some(format!("{username}/{slug}")),
        run_sh,
        json!({
            "flavor": flavor.id(),
            "timeout_seconds": timeout_secs,
            "url": format!("https://www.kaggle.com/code/{username}/{slug}"),
        }),
    ))
}

pub async fn gcp_launch_spec(
    args: &crate::ExpRunArgs,
    source: &SourceSnapshot,
    run_id: &str,
    command: &str,
) -> Result<LaunchSpec> {
    let controller = crate::local::gcp::controller_spec(args, source, run_id, command).await?;
    let orx = std::env::current_exe()
        .and_then(crate::paths::canonicalize)
        .map_err(|e| anyhow!("Could not find the orx executable for the Google Cloud run: {e}"))?;
    let files = [(
        crate::local::gcp::CONTROLLER_FILE.to_string(),
        serde_json::to_string_pretty(&controller)?,
    )];
    let run_sh = materialize(
        &files,
        &format!(
            "exec {} gcp-run {}",
            sh_quote(&crate::local::bash::bash_path(&orx)),
            sh_quote(crate::local::gcp::CONTROLLER_FILE)
        ),
    );
    let mut launch = spec(
        "gcp_job",
        Some(controller.project.clone()),
        run_sh,
        json!({ "flavor": controller.flavor, "timeout_seconds": controller.timeout_secs }),
    );
    launch.options["cancel_sh"] = crate::local::gcp::cancel_script(&controller)?.into();
    Ok(launch)
}

/// The descriptor a run shows, for an engine run as for a 0.2 one, so code that
/// asks which backend a run used finds controller runs either way.
pub fn run_backend(store: &Store, run: &StoredRun) -> Option<Value> {
    let launch = store
        .engine_run_state(&run.id)
        .ok()
        .flatten()
        .filter(|_| crate::engine::is_engine_row(run))
        .and_then(|state| state.launch);
    crate::store::public_backend(&run.backend_json, launch.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_run_script_writes_every_file_verbatim_and_runs_the_controller() {
        let dir = std::env::temp_dir().join(format!("orx-materialize-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tricky = "it's \"quoted\" $HOME `date` \\ %s\nEOF\n".to_string();
        let files = [
            ("a/b/c.txt".to_string(), tricky.clone()),
            ("top.sh".to_string(), "echo ran-$1".to_string()),
        ];
        let script = materialize(&files, "exec bash top.sh ok");
        let output = crate::process::command("bash")
            .arg("-c")
            .arg(&script)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ran-ok");
        assert_eq!(
            std::fs::read_to_string(dir.join("a/b/c.txt")).unwrap(),
            tricky
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_controller_run_shows_its_backend_flavor_and_link() {
        let launch = spec(
            "kaggle_job",
            Some("me/orx-run".into()),
            "true".into(),
            json!({ "flavor": "t4", "timeout_seconds": 60, "url": "https://k", "missing": null }),
        );
        assert!(launch.options.get("missing").is_none());
        let public = crate::store::public_backend(
            r#"{"kind":"engine","handle":{"type":"Local","dir":"/r"}}"#,
            Some(&launch),
        )
        .unwrap();
        assert_eq!(public["kind"], "kaggle_job");
        assert_eq!(public["namespace"], "me/orx-run");
        assert_eq!(public["flavor"], "t4");
        assert_eq!(public["timeoutSecs"], 60);
        assert_eq!(public["url"], "https://k");
    }
}
