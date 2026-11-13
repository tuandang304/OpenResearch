//! Google Cloud runs, owned by the tuandang304 fork. The run is a local engine
//! run whose payload is `orx gcp-run`: it creates a Compute Engine VM for this
//! one run, waits for it, stages the source and starts the command over SSH,
//! follows its log, and deletes the VM at the end. A cancel deletes the VM
//! through the run's `cancel.sh` before the controller is stopped; Compute
//! Engine also deletes the VM on its own an hour past the run's timeout, and
//! `orx up` deletes any left behind by an ended run (`jobs::gcp::reap_orphans`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::compute::SourceSnapshot;
use crate::error::{anyhow, Result};
use crate::jobs::gcp;
use crate::jobs::ssh::{sh_quote, SshTarget};

/// The flavor a launch without `--flavor` gets: the cheapest GPU.
pub const DEFAULT_FLAVOR: &str = "t4";
/// The controller's spec, written into the run dir by `run.sh`.
pub const CONTROLLER_FILE: &str = "gcp-run.json";
/// The zone the VM is being created in, for `cancel.sh`.
const ZONE_FILE: &str = "gcp-zone";
/// Retries for staging and starting, while sshd and the key agent come up.
const LAUNCH_BACKOFF_SECS: [u64; 9] = [0, 10, 15, 20, 30, 30, 45, 60, 60];
const POLL: Duration = Duration::from_secs(5);
/// One SSH call; the Google Cloud and SSH clients have no timeout of their own.
const SSH_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Consecutive failed polls before the run is given up as unreachable.
const MAX_POLL_FAILURES: u32 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerSpec {
    pub run_id: String,
    pub project: String,
    pub zone: String,
    pub flavor: String,
    pub timeout_secs: u64,
    pub archive: PathBuf,
    pub command: String,
}

impl ControllerSpec {
    fn name(&self) -> String {
        gcp::instance_name(&self.run_id)
    }
}

/// Check the launch and resolve where the VM goes; nothing is created yet.
pub async fn controller_spec(
    args: &crate::ExpRunArgs,
    source: &SourceSnapshot,
    run_id: &str,
    command: &str,
) -> Result<ControllerSpec> {
    if args.image.is_some() {
        return Err(anyhow!(
            "--image doesn't apply to --backend gcp. Set the VM image family in the Google \
             Cloud compute settings; install project dependencies in the run command."
        ));
    }
    let flavor = args
        .flavor
        .clone()
        .unwrap_or_else(|| DEFAULT_FLAVOR.to_string());
    gcp::parse_flavor(&flavor)?;
    let timeout_secs = match &args.timeout {
        Some(t) => crate::jobs::huggingface::parse_timeout(t)?,
        None => 4 * 3600,
    };
    let settings = gcp::load_settings()?.unwrap_or_default();
    let (project, zone) = gcp::resolve_location(&settings).await;
    let project = project.ok_or_else(|| {
        anyhow!(
            "No Google Cloud project. Run `orx compute configure gcp --project <id>` or \
             `gcloud config set project <id>`."
        )
    })?;
    gcp::ensure_ssh_key().await?;
    Ok(ControllerSpec {
        run_id: run_id.to_string(),
        project,
        zone,
        flavor,
        timeout_secs,
        archive: source.path.clone(),
        command: command.to_string(),
    })
}

/// Deletes the VM in whichever zone the controller last tried, without waiting.
pub fn cancel_script(spec: &ControllerSpec) -> Result<String> {
    let cli = gcp::find_cli().ok_or_else(|| anyhow!("{}", gcp::INSTALL_HINT))?;
    Ok(format!(
        "zone=$(cat {zone_file} 2>/dev/null) || exit 0\n\
         [ -n \"$zone\" ] || exit 0\n\
         {cli} compute instances delete {name} --project={project} --zone=\"$zone\" --delete-disks=all --quiet --async </dev/null >/dev/null 2>&1 || true\n",
        zone_file = sh_quote(ZONE_FILE),
        cli = sh_quote(&crate::local::bash::bash_path(&cli)),
        name = sh_quote(&spec.name()),
        project = sh_quote(&spec.project),
    ))
}

/// `orx gcp-run <spec>`: the run's payload. Returns the run's exit code.
pub async fn run_controller(spec_path: &Path) -> Result<i32> {
    let spec: ControllerSpec = serde_json::from_slice(
        &std::fs::read(spec_path)
            .map_err(|e| anyhow!("Could not read {}: {e}", spec_path.display()))?,
    )?;
    let zone = std::sync::Mutex::new(None::<String>);
    let outcome = tokio::select! {
        outcome = drive(&spec, &zone) => outcome,
        () = stopped() => {
            println!("[orx] Stopping; deleting Google Cloud VM {}", spec.name());
            Ok(143)
        }
    };
    let code = match outcome {
        Ok(code) => code,
        Err(error) => {
            println!("[orx] {error}");
            1
        }
    };
    let zone = zone.lock().unwrap_or_else(|p| p.into_inner()).clone();
    if let Some(zone) = zone {
        teardown(&spec, &zone).await;
    }
    Ok(code)
}

async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            return std::future::pending().await;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = hup.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn drive(spec: &ControllerSpec, zone_cell: &std::sync::Mutex<Option<String>>) -> Result<i32> {
    let name = spec.name();
    let shape = gcp::parse_flavor(&spec.flavor)?;
    let settings = gcp::load_settings()?.unwrap_or_default();
    let public_key = gcp::ensure_ssh_key().await?;
    println!("[orx] Creating Google Cloud VM {name} ({})", spec.flavor);
    let zone = gcp::create_in_gpu_zone(
        &gcp::CreateSpec {
            project: &spec.project,
            zone: &spec.zone,
            name: &name,
            run_id: &spec.run_id,
            shape: &shape,
            settings: &settings,
            ssh_public_key: &public_key,
            timeout_secs: spec.timeout_secs,
        },
        |zone| {
            *zone_cell.lock().unwrap_or_else(|p| p.into_inner()) = Some(zone.to_string());
            std::fs::write(ZONE_FILE, zone)
                .map_err(|e| anyhow!("Could not record the VM's zone: {e}"))
        },
    )
    .await?;
    println!(
        "[orx] VM: https://console.cloud.google.com/compute/instancesDetail/zones/{zone}/instances/{name}?project={}",
        spec.project
    );
    let ip = match gcp::wait_running(&spec.project, &zone, &name, || false).await {
        gcp::WaitOutcome::Running(ip) => ip,
        gcp::WaitOutcome::Cancelled => return Ok(143),
        gcp::WaitOutcome::Failed(reason) => {
            return Err(anyhow!("Provisioning failed: {reason}."));
        }
    };
    let mut target = gcp::ssh_target(&ip)?;
    target.managed_generation = Some(name.clone());
    let target = target.for_run(&spec.run_id);
    let dir = crate::jobs::openresearch::run_dir(&spec.run_id);

    let archive = std::fs::read(&spec.archive).map_err(|e| {
        anyhow!(
            "The source snapshot {} could not be read: {e}",
            spec.archive.display()
        )
    })?;
    let mut script = crate::compute::staged_script(&spec.command);
    if shape.gpu.is_some() {
        script = gcp::wait_for_gpu(&script);
    }
    // Outside the timeout, so checkpoints are still saved after the run is cut off.
    let script = crate::jobs::checkpoints::wrap(&crate::jobs::openresearch::wrap_with_timeout(
        &script,
        spec.timeout_secs,
    ));
    let mut env = crate::config::launch_env(&spec.run_id);
    env.extend(
        crate::jobs::checkpoints::run_env(&spec.run_id)
            .map_err(|e| anyhow!("Checkpoint storage cannot be used: {e}"))?,
    );
    let mut payload: String = env
        .iter()
        .filter(|(key, _)| is_env_name(key))
        .map(|(key, value)| format!("export {key}={}\n", sh_quote(value)))
        .collect();
    payload.push_str(&script);
    payload.push('\n');

    let d = sh_quote(&dir);
    let stage = format!("umask 077; mkdir -p {d}/repo && tar -xf - -C {d}/repo");
    let start = format!(
        "cd {d} && umask 077 && cat > run.sh && \
         setsid nohup bash -c 'bash run.sh > log 2>&1; echo $? > exit_code.part; mv exit_code.part exit_code' </dev/null >/dev/null 2>&1 &"
    );
    let mut launch_error = None;
    for backoff in LAUNCH_BACKOFF_SECS {
        if backoff > 0 {
            tokio::time::sleep(crate::config::paced(Duration::from_secs(backoff))).await;
        }
        let attempt = async {
            ssh(&target, &stage, Some(&archive)).await?;
            ssh(&target, &start, Some(payload.as_bytes())).await
        };
        match attempt.await {
            Ok(_) => {
                launch_error = None;
                break;
            }
            Err(error) => {
                println!("[orx] Could not start the run on the VM yet (will retry): {error}");
                launch_error = Some(error);
            }
        }
    }
    if let Some(error) = launch_error {
        let error = error.to_string();
        let hint = if error.contains("Permission denied") || error.contains("publickey") {
            " If the project enforces OS Login through an organization policy, orx's SSH key is \
             ignored; use another project or the ssh backend."
        } else {
            ""
        };
        return Err(anyhow!(
            "Could not start the run on Google Cloud VM {name}: {error}{hint}"
        ));
    }
    println!("[orx] Running on {name} ({ip})");
    follow(&target, &dir, spec.timeout_secs).await
}

/// Stream the remote log until the command records its exit code.
async fn follow(target: &SshTarget, dir: &str, timeout_secs: u64) -> Result<i32> {
    use std::io::Write as _;
    const MARK: &str = "\n--orx-log--\n";
    let d = sh_quote(dir);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs + 15 * 60);
    let mut offset: u64 = 0;
    let mut failures = 0;
    loop {
        let poll = format!(
            "cd {d} && {{ cat exit_code 2>/dev/null; printf '%s' {mark}; tail -c +{} log 2>/dev/null; }}",
            offset + 1,
            mark = sh_quote(MARK),
        );
        match ssh(target, &poll, None).await {
            Ok(output) => {
                failures = 0;
                let (code, log) = output.split_once(MARK).unwrap_or(("", ""));
                if !log.is_empty() {
                    offset += log.len() as u64;
                    let mut stdout = std::io::stdout().lock();
                    let _ = stdout.write_all(log.as_bytes());
                    let _ = stdout.flush();
                }
                if let Ok(code) = code.trim().parse::<i32>() {
                    return Ok(code);
                }
            }
            Err(error) => {
                failures += 1;
                if failures >= MAX_POLL_FAILURES {
                    return Err(anyhow!("Lost contact with the VM: {error}"));
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "The run did not finish within its {timeout_secs}s timeout."
            ));
        }
        tokio::time::sleep(crate::config::paced(POLL)).await;
    }
}

async fn ssh(target: &SshTarget, command: &str, stdin: Option<&[u8]>) -> Result<String> {
    tokio::time::timeout(
        SSH_CALL_TIMEOUT,
        crate::jobs::ssh::direct_run_bytes(target, command, stdin),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "ssh {} did not answer within {}s",
            target.dest,
            SSH_CALL_TIMEOUT.as_secs()
        )
    })?
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Delete the run's VM; on failure say how to delete it by hand.
async fn teardown(spec: &ControllerSpec, zone: &str) {
    let name = spec.name();
    match gcp::delete(&spec.project, zone, &name).await {
        Ok(()) => println!("[orx] Google Cloud VM {name} deleted"),
        Err(error) => println!(
            "[orx] Warning: Google Cloud VM {name} could not be deleted ({error}) and is still \
             billing. Delete it with `gcloud compute instances delete {name} --project={} \
             --zone={zone}`. Compute Engine deletes it on its own once the run's time limit \
             plus one hour has passed.",
            spec.project
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_round_trips_through_the_run_dir_file() {
        let spec = ControllerSpec {
            run_id: "run".into(),
            project: "proj".into(),
            zone: "us-central1-a".into(),
            flavor: "t4".into(),
            timeout_secs: 60,
            archive: "/sources/a.tar".into(),
            command: "python train.py".into(),
        };
        let text = serde_json::to_string(&spec).unwrap();
        let back: ControllerSpec = serde_json::from_str(&text).unwrap();
        assert_eq!(back.name(), gcp::instance_name("run"));
        assert_eq!(back.archive, spec.archive);
    }
}
