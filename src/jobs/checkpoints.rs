//! Checkpoint storage for machines that disappear with the run (Google Cloud VMs, Modal
//! Sandboxes): the run writes to `$ORX_CHECKPOINT_DIR`, which syncs to a remote the user
//! set up with rclone (Google Drive, a GCS bucket, S3, …) while it runs and once more when
//! it ends, and is restored from there when the same experiment runs again.
//!
//! The remote's credentials come from the user's own rclone config and travel to the run as
//! `RCLONE_CONFIG_<NAME>_<KEY>` variables, never through a stored launch intent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{anyhow, Context, Result};

/// The name the run's rclone knows the remote by, whatever the user called it.
const REMOTE_NAME: &str = "ORXCKPT";
pub const DEFAULT_INTERVAL_MINUTES: u64 = 10;
/// Installed on the run machine when its image has no rclone.
const RCLONE_VERSION: &str = "v1.75.0";

pub const REMOTE_ENV: &str = "ORX_CHECKPOINT_REMOTE";
const LABEL_ENV: &str = "ORX_CHECKPOINT_LABEL";
const INTERVAL_ENV: &str = "ORX_CHECKPOINT_INTERVAL";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointSettings {
    /// An rclone remote and folder, such as `gdrive:openresearch`. Each experiment gets
    /// `<remote>/<project>/<experiment>` under it.
    pub remote: String,
    #[serde(default)]
    pub interval_minutes: Option<u64>,
    /// The rclone config file the remote was found in when it was saved; rclone's default
    /// locations otherwise. Recorded because the app may not see the shell's `RCLONE_CONFIG`.
    #[serde(default)]
    pub rclone_config: Option<PathBuf>,
}

impl CheckpointSettings {
    pub fn interval_minutes(&self) -> u64 {
        self.interval_minutes.unwrap_or(DEFAULT_INTERVAL_MINUTES)
    }

    fn remote_name(&self) -> &str {
        self.remote.split_once(':').map_or("", |(name, _)| name)
    }

    fn remote_path(&self) -> &str {
        self.remote
            .split_once(':')
            .map_or("", |(_, path)| path.trim_matches('/'))
    }
}

fn settings_path() -> PathBuf {
    crate::config::config_dir().join("checkpoints.json")
}

pub fn load_settings() -> Result<Option<CheckpointSettings>> {
    let path = settings_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow!("Could not read {}: {e}", path.display())),
    };
    serde_json::from_str(&raw).map(Some).map_err(|e| {
        anyhow!(
            "Unreadable {} ({e}). Fix or delete it, then run `orx compute checkpoints set`.",
            path.display()
        )
    })
}

/// Checks the remote against the rclone config and saves it.
pub fn save_settings(
    remote: &str,
    interval_minutes: Option<u64>,
    rclone_config: Option<PathBuf>,
) -> Result<CheckpointSettings> {
    let remote = remote.trim();
    let Some((name, _)) = remote.split_once(':') else {
        return Err(anyhow!(
            "'{remote}' is not an rclone remote. Use the remote's name, a colon and a folder, \
             such as gdrive:openresearch."
        ));
    };
    if name.is_empty() {
        return Err(anyhow!(
            "'{remote}' has no remote name before the colon, such as gdrive:openresearch."
        ));
    }
    if let Some(minutes) = interval_minutes {
        if !(1..=24 * 60).contains(&minutes) {
            return Err(anyhow!("The sync interval must be 1–1440 minutes."));
        }
    }
    let config = rclone_config
        .or_else(default_rclone_config)
        .ok_or_else(|| {
            anyhow!(
                "No rclone config found. Install rclone and run `rclone config` to add a remote."
            )
        })?;
    let settings = CheckpointSettings {
        remote: remote.to_string(),
        interval_minutes,
        rclone_config: Some(config),
    };
    remote_section(&settings)?;
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&settings)?),
    )
    .with_context(|| format!("Could not write {}", path.display()))?;
    Ok(settings)
}

pub fn clear_settings() -> Result<()> {
    let path = settings_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!("Could not delete {}: {e}", path.display())),
    }
}

/// rclone's own lookup order: `$RCLONE_CONFIG`, then the platform config dir, then the
/// legacy `~/.rclone.conf`.
fn default_rclone_config() -> Option<PathBuf> {
    if let Some(path) = crate::local::shell_env::var("RCLONE_CONFIG") {
        return Some(PathBuf::from(path));
    }
    let home = crate::config::user_home();
    let mut candidates = Vec::new();
    if cfg!(windows) {
        if let Some(appdata) = crate::local::shell_env::var("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("rclone").join("rclone.conf"));
        }
    } else if let Some(xdg) = crate::local::shell_env::var("XDG_CONFIG_HOME") {
        candidates.push(PathBuf::from(xdg).join("rclone").join("rclone.conf"));
    }
    if let Some(home) = &home {
        candidates.push(home.join(".config").join("rclone").join("rclone.conf"));
        candidates.push(home.join(".rclone.conf"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// The remote's `key = value` lines from the rclone config.
fn remote_section(settings: &CheckpointSettings) -> Result<Vec<(String, String)>> {
    let name = settings.remote_name();
    let path = settings
        .rclone_config
        .clone()
        .or_else(default_rclone_config)
        .ok_or_else(|| {
            anyhow!(
                "No rclone config found for checkpoint storage {}. Run `rclone config` to add \
                 the remote, or `orx compute checkpoints clear` to stop saving checkpoints.",
                settings.remote
            )
        })?;
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        anyhow!(
            "Could not read the rclone config {} for checkpoint storage {}: {e}",
            path.display(),
            settings.remote
        )
    })?;
    parse_section(&raw, name).map_err(|e| anyhow!("{e} ({})", path.display()))
}

fn parse_section(raw: &str, name: &str) -> Result<Vec<(String, String)>> {
    if raw
        .trim_start()
        .starts_with("# Encrypted rclone configuration File")
    {
        return Err(anyhow!(
            "The rclone config is encrypted, so orx cannot pass remote '{name}' to the run. \
             Remove its password with `rclone config` (s, then r) or use an unencrypted config"
        ));
    }
    let mut section: Option<Vec<(String, String)>> = None;
    let mut current = false;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = header.trim() == name;
            if current {
                section = Some(Vec::new());
            }
            continue;
        }
        if !current {
            continue;
        }
        if let (Some(fields), Some((key, value))) = (section.as_mut(), line.split_once('=')) {
            fields.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    let fields = section.ok_or_else(|| {
        anyhow!("rclone has no remote named '{name}'. Add it with `rclone config`")
    })?;
    if !fields.iter().any(|(key, _)| key == "type") {
        return Err(anyhow!(
            "rclone remote '{name}' has no type. Set it up again with `rclone config`"
        ));
    }
    // A wrapper (crypt, alias, union, …) names another remote the run would also need.
    if let Some((_, inner)) = fields
        .iter()
        .find(|(key, value)| matches!(key.as_str(), "remote" | "upstreams") && value.contains(':'))
    {
        return Err(anyhow!(
            "rclone remote '{name}' wraps another remote ({inner}); checkpoint storage needs a \
             remote that stores files itself, such as Google Drive, Google Cloud Storage or S3"
        ));
    }
    Ok(fields)
}

/// What a run on a Google Cloud VM or a Modal Sandbox needs for checkpoint storage: the
/// remote's credentials, where this experiment's checkpoints live, and how often to sync.
/// Empty when checkpoint storage is off; an error when it is on but cannot be used, so a
/// run never trains for hours with nowhere to save.
pub fn run_env(run_id: &str) -> Result<HashMap<String, String>> {
    let Some(settings) = load_settings()? else {
        return Ok(HashMap::new());
    };
    let section = remote_section(&settings)?;
    let store = crate::store::Store::open()?;
    let run = store
        .get_run(run_id)?
        .ok_or_else(|| anyhow!("Run {run_id} not found."))?;
    let experiment = store
        .get_local_experiment(&run.experiment_id)?
        .ok_or_else(|| anyhow!("Local experiment {} not found.", run.experiment_id))?;
    let project = store
        .get_local_project(&run.project_id)?
        .ok_or_else(|| anyhow!("Local project {} not found.", run.project_id))?;
    Ok(env_for(
        &settings,
        &section,
        &project.slug,
        &experiment.slug,
    ))
}

fn env_for(
    settings: &CheckpointSettings,
    section: &[(String, String)],
    project: &str,
    experiment: &str,
) -> HashMap<String, String> {
    let folder = [settings.remote_path(), project, experiment]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    let mut env: HashMap<String, String> = section
        .iter()
        .map(|(key, value)| {
            (
                format!(
                    "RCLONE_CONFIG_{REMOTE_NAME}_{}",
                    key.to_ascii_uppercase().replace('-', "_")
                ),
                value.clone(),
            )
        })
        .collect();
    env.insert(
        REMOTE_ENV.to_string(),
        format!("{}:{folder}", REMOTE_NAME.to_ascii_lowercase()),
    );
    env.insert(
        LABEL_ENV.to_string(),
        format!("{}:{folder}", settings.remote_name()),
    );
    env.insert(
        INTERVAL_ENV.to_string(),
        (settings.interval_minutes() * 60).to_string(),
    );
    env
}

/// Runs `script` with `$ORX_CHECKPOINT_DIR` set. With checkpoint storage on, it restores the
/// experiment's earlier checkpoints first, syncs in the background, and saves once more after
/// `script` exits, keeping its exit code. Off, it says where checkpoints go and that they are
/// lost with the machine.
pub fn wrap(script: &str) -> String {
    let script = super::ssh::sh_quote(script);
    let installer = super::ssh::sh_quote(INSTALLER);
    format!(
        r#"export ORX_CHECKPOINT_DIR="${{ORX_CHECKPOINT_DIR:-${{HOME:-/tmp}}/orx-checkpoints}}"
mkdir -p "$ORX_CHECKPOINT_DIR"
orx_ckpt_pid=
if [ -n "${{{REMOTE_ENV}:-}}" ]; then
  orx_ckpt_label="${{{LABEL_ENV}:-${REMOTE_ENV}}}"
  orx_ckpt_every="${{{INTERVAL_ENV}:-600}}"
  if ! command -v rclone >/dev/null 2>&1; then
    orx_ckpt_bin="${{HOME:-/tmp}}/.orx-rclone"
    if [ ! -x "$orx_ckpt_bin/rclone" ]; then
      echo "[orx] Installing rclone {RCLONE_VERSION} for checkpoint storage"
      case "$(uname -m)" in aarch64|arm64) orx_ckpt_arch=arm64 ;; *) orx_ckpt_arch=amd64 ;; esac
      mkdir -p "$orx_ckpt_bin"
      if ! python3 -c {installer} "$orx_ckpt_bin" "https://downloads.rclone.org/{RCLONE_VERSION}/rclone-{RCLONE_VERSION}-linux-$orx_ckpt_arch.zip"; then
        echo "[orx] Could not install rclone, so checkpoints cannot be saved to $orx_ckpt_label. The run stops before training; install rclone in the image or turn checkpoint storage off with \`orx compute checkpoints clear\`."
        exit 1
      fi
    fi
    export PATH="$orx_ckpt_bin:$PATH"
  fi
  rclone copy "${REMOTE_ENV}" "$ORX_CHECKPOINT_DIR" --retries 1 --log-level ERROR
  orx_ckpt_rc=$?
  if [ "$orx_ckpt_rc" = 0 ]; then
    echo "[orx] Restored $(find "$ORX_CHECKPOINT_DIR" -type f | wc -l | tr -d ' ') checkpoint files from $orx_ckpt_label into \$ORX_CHECKPOINT_DIR"
  elif [ "$orx_ckpt_rc" = 3 ]; then
    echo "[orx] No earlier checkpoints at $orx_ckpt_label; starting fresh"
  else
    echo "[orx] Could not reach checkpoint storage $orx_ckpt_label (rclone exit $orx_ckpt_rc). The run stops before training. Check the remote with \`rclone lsd ${{orx_ckpt_label%%:*}}:\` and \`rclone config reconnect ${{orx_ckpt_label%%:*}}:\` on your machine, or turn checkpoint storage off with \`orx compute checkpoints clear\`."
    exit 1
  fi
  echo "[orx] Files in \$ORX_CHECKPOINT_DIR ($ORX_CHECKPOINT_DIR) sync to $orx_ckpt_label every $((orx_ckpt_every / 60)) min and when the run ends"
  orx_ckpt_stop="$(mktemp)"
  rm -f "$orx_ckpt_stop"
  (
    while :; do
      orx_ckpt_waited=0
      while [ "$orx_ckpt_waited" -lt "$orx_ckpt_every" ]; do
        [ -e "$orx_ckpt_stop" ] && exit 0
        sleep 1
        orx_ckpt_waited=$((orx_ckpt_waited + 1))
      done
      rclone copy "$ORX_CHECKPOINT_DIR" "${REMOTE_ENV}" --log-level ERROR \
        || echo "[orx] Checkpoint sync to $orx_ckpt_label failed; retrying in $((orx_ckpt_every / 60)) min"
    done
  ) &
  orx_ckpt_pid=$!
else
  echo "[orx] Checkpoint storage is off: files in \$ORX_CHECKPOINT_DIR are deleted with this machine. Turn it on with \`orx compute checkpoints set <rclone-remote:folder>\`."
fi
bash -c {script}
orx_rc=$?
if [ -n "$orx_ckpt_pid" ]; then
  touch "$orx_ckpt_stop"
  wait "$orx_ckpt_pid"
  rm -f "$orx_ckpt_stop"
  echo "[orx] Saving checkpoints to $orx_ckpt_label"
  orx_ckpt_saved=
  for orx_ckpt_try in 1 2 3; do
    if rclone copy "$ORX_CHECKPOINT_DIR" "${REMOTE_ENV}" --log-level ERROR; then
      orx_ckpt_saved=1
      break
    fi
    [ "$orx_ckpt_try" = 3 ] || sleep $((orx_ckpt_try * 10))
  done
  if [ -n "$orx_ckpt_saved" ]; then
    echo "[orx] Checkpoints saved to $orx_ckpt_label"
  else
    echo "[orx] Checkpoints could not be saved to $orx_ckpt_label after 3 tries; only the last successful sync is kept there."
  fi
fi
exit $orx_rc"#
    )
}

/// Downloads rclone's release zip and extracts the binary: argv[1] is the directory, argv[2]
/// the URL. Python because CUDA images often lack curl and unzip.
const INSTALLER: &str = r#"import io, os, sys, urllib.request, zipfile
dest, url = sys.argv[1], sys.argv[2]
# rclone's download host refuses Python's default User-Agent.
request = urllib.request.Request(url, headers={"User-Agent": "orx-checkpoints"})
data = urllib.request.urlopen(request, timeout=300).read()
with zipfile.ZipFile(io.BytesIO(data)) as archive:
    name = next(n for n in archive.namelist() if n.endswith("/rclone"))
    path = os.path.join(dest, "rclone")
    with open(path, "wb") as out:
        out.write(archive.read(name))
os.chmod(path, 0o755)
"#;

/// The settings as the CLI and the dashboard show them.
pub fn describe(settings: Option<&CheckpointSettings>) -> serde_json::Value {
    match settings {
        None => serde_json::json!({"enabled": false}),
        Some(settings) => {
            let problem = remote_section(settings).err().map(|e| e.to_string());
            serde_json::json!({
                "enabled": true,
                "remote": settings.remote,
                "intervalMinutes": settings.interval_minutes(),
                "rcloneConfig": settings.rclone_config.as_deref().map(Path::display).map(|p| p.to_string()),
                "error": problem,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "[other]\ntype = s3\n\n[gdrive]\ntype = drive\nscope = drive.file\ntoken = {\"access_token\":\"a\",\"expiry\":\"2026-01-01\"}\nroot_folder_id = \n";

    fn settings(remote: &str) -> CheckpointSettings {
        CheckpointSettings {
            remote: remote.into(),
            interval_minutes: Some(5),
            rclone_config: None,
        }
    }

    #[test]
    fn a_remote_becomes_rclone_env_for_the_experiment_folder() {
        let section = parse_section(CONFIG, "gdrive").unwrap();
        let env = env_for(
            &settings("gdrive:/openresearch/"),
            &section,
            "proj",
            "exp-1",
        );
        assert_eq!(env["RCLONE_CONFIG_ORXCKPT_TYPE"], "drive");
        assert_eq!(env["RCLONE_CONFIG_ORXCKPT_SCOPE"], "drive.file");
        assert_eq!(
            env["RCLONE_CONFIG_ORXCKPT_TOKEN"],
            "{\"access_token\":\"a\",\"expiry\":\"2026-01-01\"}"
        );
        assert_eq!(env[REMOTE_ENV], "orxckpt:openresearch/proj/exp-1");
        assert_eq!(env[LABEL_ENV], "gdrive:openresearch/proj/exp-1");
        assert_eq!(env[INTERVAL_ENV], "300");
        assert!(!env.keys().any(|key| key.contains("OTHER")));

        let root = env_for(&settings("gdrive:"), &section, "proj", "exp-1");
        assert_eq!(root[REMOTE_ENV], "orxckpt:proj/exp-1");
    }

    #[test]
    fn unusable_remotes_are_named() {
        let missing = parse_section(CONFIG, "dropbox").unwrap_err().to_string();
        assert!(missing.contains("dropbox"), "{missing}");
        let encrypted = parse_section(
            "# Encrypted rclone configuration File\n\nRCLONE_ENCRYPT_V0:\nabc",
            "gdrive",
        )
        .unwrap_err()
        .to_string();
        assert!(encrypted.contains("encrypted"), "{encrypted}");
        let wrapper = parse_section("[secret]\ntype = crypt\nremote = gdrive:vault\n", "secret")
            .unwrap_err()
            .to_string();
        assert!(wrapper.contains("gdrive:vault"), "{wrapper}");
    }

    #[test]
    fn saving_checks_the_remote_and_round_trips() {
        // Alone in a child, so no other test's launch reads these settings.
        if !crate::config::in_test_child(&[]) {
            return;
        }
        let dir = crate::config::test_home().join("rclone-save");
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("rclone.conf");
        std::fs::write(&conf, CONFIG).unwrap();
        assert!(save_settings("gdrive", None, Some(conf.clone())).is_err());
        assert!(save_settings("nope:x", None, Some(conf.clone())).is_err());
        assert!(save_settings("gdrive:x", Some(0), Some(conf.clone())).is_err());
        let saved = save_settings(" gdrive:runs ", Some(15), Some(conf.clone())).unwrap();
        assert_eq!(load_settings().unwrap(), Some(saved.clone()));
        assert_eq!(describe(Some(&saved))["error"], serde_json::Value::Null);
        std::fs::remove_file(&conf).unwrap();
        assert!(describe(Some(&saved))["error"].is_string());
        clear_settings().unwrap();
        assert_eq!(load_settings().unwrap(), None);
    }

    /// Runs the wrapper with a fake rclone that treats `orxckpt:<path>` as a local folder.
    fn run_wrapped(payload: &str, remote: Option<&Path>, root: &Path) -> (i32, String) {
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("rclone");
        std::fs::write(
            &fake,
            "#!/usr/bin/env bash\nset -e\n[ -e \"$HOME/fail\" ] && { echo 'token expired' >&2; exit 1; }\nmap() { case \"$1\" in orxckpt:*) echo \"$FAKE_REMOTE/${1#orxckpt:}\" ;; *) echo \"$1\" ;; esac; }\nsrc=\"$(map \"$2\")\"; dst=\"$(map \"$3\")\"\n[ -d \"$src\" ] || exit 3\nmkdir -p \"$dst\"; cp -R \"$src/.\" \"$dst/\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut command = crate::process::command("bash");
        command
            .arg("-c")
            .arg(wrap(payload))
            .env("HOME", root)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env_remove("ORX_CHECKPOINT_DIR");
        if let Some(remote) = remote {
            command
                .env("FAKE_REMOTE", remote)
                .env(REMOTE_ENV, "orxckpt:proj/exp")
                .env(LABEL_ENV, "gdrive:proj/exp")
                .env(INTERVAL_ENV, "1");
        }
        let output = command.output().unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    fn have_bash() -> bool {
        if cfg!(windows) {
            return false;
        }
        let found = crate::process::command("bash")
            .arg("-c")
            .arg("true")
            .status()
            .is_ok_and(|status| status.success());
        if !found {
            crate::config::skip_without_tool("bash");
        }
        found
    }

    #[test]
    fn a_run_restores_syncs_and_saves_its_checkpoints() {
        if !have_bash() {
            return;
        }
        let root = crate::config::test_home().join("ckpt-sync");
        let remote = root.join("remote");
        std::fs::create_dir_all(remote.join("proj/exp")).unwrap();
        std::fs::write(remote.join("proj/exp/step-1.pt"), "old").unwrap();

        let (code, out) = run_wrapped(
            "test \"$(cat \"$ORX_CHECKPOINT_DIR/step-1.pt\")\" = old || exit 9; echo new > \"$ORX_CHECKPOINT_DIR/step-2.pt\"; for i in $(seq 150); do [ -f \"$FAKE_REMOTE/proj/exp/step-2.pt\" ] && exit 4; sleep 0.2; done; exit 5",
            Some(&remote),
            &root,
        );
        assert_eq!(
            code, 4,
            "the background sync uploads during the run and the exit code is kept:\n{out}"
        );
        assert!(
            out.contains("Restored 1 checkpoint files from gdrive:proj/exp"),
            "{out}"
        );
        assert!(
            out.contains("Checkpoints saved to gdrive:proj/exp"),
            "{out}"
        );
        assert_eq!(
            std::fs::read_to_string(remote.join("proj/exp/step-2.pt")).unwrap(),
            "new\n"
        );
    }

    #[test]
    fn a_first_run_starts_fresh_and_storage_off_says_so() {
        if !have_bash() {
            return;
        }
        let root = crate::config::test_home().join("ckpt-fresh");
        let remote = root.join("remote");
        std::fs::create_dir_all(&remote).unwrap();
        let (code, out) = run_wrapped("touch \"$ORX_CHECKPOINT_DIR/a.pt\"", Some(&remote), &root);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("No earlier checkpoints"), "{out}");
        assert!(remote.join("proj/exp/a.pt").is_file(), "{out}");

        let (code, out) = run_wrapped("test -d \"$ORX_CHECKPOINT_DIR\"", None, &root);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("Checkpoint storage is off"), "{out}");
    }

    #[test]
    fn an_unreachable_remote_stops_the_run_before_training() {
        if !have_bash() {
            return;
        }
        let root = crate::config::test_home().join("ckpt-broken");
        let remote = root.join("remote");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(root.join("fail"), "").unwrap();
        let (code, out) = run_wrapped("echo trained", Some(&remote), &root);
        assert_eq!(code, 1, "{out}");
        assert!(
            out.contains("Could not reach checkpoint storage gdrive:proj/exp"),
            "{out}"
        );
        assert!(!out.contains("trained"), "{out}");
    }
}
