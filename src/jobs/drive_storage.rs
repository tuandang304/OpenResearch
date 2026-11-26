//! Google Drive storage, owned by the tuandang304 fork: a Drive folder the user links and
//! turns on, where an agent saves datasets, checkpoints and other large outputs with
//! `orx compute drive push|pull|ls`. Off until the user turns it on, since Drive is slow.
//!
//! Transfers go through the user's own rclone and its Google Drive remote, pinned to the
//! linked folder with `root_folder_id`, so every path stays inside that folder.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::checkpoints::{default_rclone_config, parse_section};
use crate::error::{anyhow, Context, Result};

const INSTALL_HINT: &str = "Install rclone (https://rclone.org/install/), then add a Google \
     Drive remote with `rclone config` (storage type drive, scope drive).";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DriveSettings {
    /// Off by default; only the user turns it on, in Settings or with `orx compute drive on`.
    #[serde(default)]
    pub enabled: bool,
    /// The folder link as the user gave it.
    pub folder_url: String,
    pub folder_id: String,
    /// The rclone remote (type drive) that reaches the folder.
    pub remote: String,
    /// The rclone config file the remote was found in when it was saved.
    #[serde(default)]
    pub rclone_config: Option<PathBuf>,
}

impl DriveSettings {
    /// rclone's name for `path` inside the linked folder.
    fn target(&self, path: &str) -> String {
        format!("{},root_folder_id={}:{path}", self.remote, self.folder_id)
    }
}

fn settings_path() -> PathBuf {
    crate::config::config_dir().join("drive-storage.json")
}

pub fn load_settings() -> Result<Option<DriveSettings>> {
    let path = settings_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow!("Could not read {}: {e}", path.display())),
    };
    serde_json::from_str(&raw).map(Some).map_err(|e| {
        anyhow!(
            "Unreadable {} ({e}). Fix or delete it, then run `orx compute drive set <folder-link>`.",
            path.display()
        )
    })
}

fn write_settings(settings: &DriveSettings) -> Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(settings)?),
    )
    .with_context(|| format!("Could not write {}", path.display()))
}

pub fn clear_settings() -> Result<()> {
    let path = settings_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!("Could not delete {}: {e}", path.display())),
    }
}

/// The folder id in a Drive folder link (`…/drive/folders/<id>`, `…/drive/u/0/folders/<id>`,
/// `…open?id=<id>`) or a bare id.
pub fn folder_id(link: &str) -> Result<String> {
    let link = link.trim();
    let candidate = if let Some((_, rest)) = link.split_once("/folders/") {
        rest
    } else if let Some((_, rest)) = link.split_once("id=") {
        rest
    } else if link.contains('/') || link.contains('.') {
        ""
    } else {
        link
    };
    let id: String = candidate
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if id.len() < 10 {
        return Err(anyhow!(
            "'{link}' is not a Google Drive folder link. Open the folder in Drive and copy its \
             link, such as https://drive.google.com/drive/folders/<id>."
        ));
    }
    Ok(id)
}

/// Names of the Google Drive remotes in an rclone config.
fn drive_remotes(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix('[')
                .and_then(|l| l.strip_suffix(']'))
        })
        .map(|name| name.trim().to_string())
        .filter(|name| {
            parse_section(raw, name)
                .is_ok_and(|fields| fields.iter().any(|(k, v)| k == "type" && v == "drive"))
        })
        .collect()
}

fn read_config(config: &Path) -> Result<String> {
    std::fs::read_to_string(config)
        .map_err(|e| anyhow!("Could not read the rclone config {}: {e}", config.display()))
}

/// Checks that `name` is a Google Drive remote that can open folders it did not create.
fn check_remote(raw: &str, name: &str, config: &Path) -> Result<()> {
    let fields = parse_section(raw, name).map_err(|e| anyhow!("{e} ({})", config.display()))?;
    let field = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    if field("type") != Some("drive") {
        return Err(anyhow!(
            "rclone remote '{name}' is not Google Drive. Pick a remote of type drive, or add one \
             with `rclone config`."
        ));
    }
    if matches!(
        field("scope"),
        Some("drive.file" | "drive.readonly" | "drive.metadata.readonly")
    ) {
        return Err(anyhow!(
            "rclone remote '{name}' cannot write to a folder rclone did not create (scope {}). \
             Run `rclone config`, edit '{name}' and set scope to drive.",
            field("scope").unwrap_or_default()
        ));
    }
    Ok(())
}

/// What the user sends from Settings or the CLI. `None` keeps the saved value.
#[derive(Debug, Default)]
pub struct Update {
    pub enabled: Option<bool>,
    pub folder: Option<String>,
    pub remote: Option<String>,
    pub rclone_config: Option<PathBuf>,
}

/// Applies `update` to the saved settings and saves them. Turning storage on also checks
/// that rclone can open the folder, so an agent never finds out halfway through an upload.
pub async fn update(update: Update) -> Result<Option<DriveSettings>> {
    let saved = load_settings()?;
    let folder = match update.folder.as_deref().map(str::trim) {
        Some("") => {
            clear_settings()?;
            return Ok(None);
        }
        Some(link) => link.to_string(),
        None => match &saved {
            Some(saved) => saved.folder_url.clone(),
            None => {
                return Err(anyhow!(
                    "No Google Drive folder is linked. Run `orx compute drive set <folder-link>` \
                     or add it under Google Drive storage in the compute settings."
                ))
            }
        },
    };
    let folder_id = folder_id(&folder)?;
    let config = update
        .rclone_config
        .or_else(|| saved.as_ref().and_then(|s| s.rclone_config.clone()))
        .or_else(default_rclone_config)
        .ok_or_else(|| anyhow!("No rclone config found. {INSTALL_HINT}"))?;
    let raw = read_config(&config)?;
    let remote = match update
        .remote
        .map(|r| r.trim().trim_end_matches(':').to_string())
        .filter(|r| !r.is_empty())
        .or_else(|| saved.as_ref().map(|s| s.remote.clone()))
    {
        Some(remote) => remote,
        None => {
            let found = drive_remotes(&raw);
            match found.as_slice() {
                [only] => only.clone(),
                [] => {
                    return Err(anyhow!(
                        "{} has no Google Drive remote. {INSTALL_HINT}",
                        config.display()
                    ))
                }
                many => {
                    return Err(anyhow!(
                        "{} has several Google Drive remotes ({}). Choose one with --remote.",
                        config.display(),
                        many.join(", ")
                    ))
                }
            }
        }
    };
    check_remote(&raw, &remote, &config)?;
    let settings = DriveSettings {
        enabled: update
            .enabled
            .unwrap_or_else(|| saved.as_ref().is_some_and(|s| s.enabled)),
        folder_url: folder,
        folder_id,
        remote,
        rclone_config: Some(config),
    };
    if settings.enabled {
        verify(&settings).await?;
    }
    write_settings(&settings)?;
    Ok(Some(settings))
}

fn rclone() -> Result<PathBuf> {
    crate::local::shell_env::find("rclone")
        .ok_or_else(|| anyhow!("rclone is not installed. {INSTALL_HINT}"))
}

fn base_command(settings: &DriveSettings) -> Result<tokio::process::Command> {
    let mut command = crate::process::tokio_command(rclone()?);
    if let Some(config) = &settings.rclone_config {
        command.arg("--config").arg(config);
    }
    Ok(command)
}

/// Lists the folder's top level, which fails when the remote cannot open it.
async fn verify(settings: &DriveSettings) -> Result<()> {
    let mut command = base_command(settings)?;
    command
        .args(["lsf", "--max-depth", "1", "--retries", "1"])
        .arg(settings.target(""));
    let output = tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .map_err(|_| {
            anyhow!(
                "rclone took over a minute to open {} with remote '{}'. Check your connection, \
                 then try again.",
                settings.folder_url,
                settings.remote
            )
        })?
        .context("Could not run rclone")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "rclone remote '{}' could not open {}: {}. Check that the Google account behind the \
         remote can edit this folder, or sign in again with `rclone config reconnect {}:`.",
        settings.remote,
        settings.folder_url,
        stderr.trim().lines().last().unwrap_or("no details"),
        settings.remote
    ))
}

/// The saved settings, or why an agent may not use Drive now.
pub fn require_enabled() -> Result<DriveSettings> {
    match load_settings()? {
        Some(settings) if settings.enabled => Ok(settings),
        Some(_) => Err(anyhow!(
            "Google Drive storage is off. Save to Drive only when the user asks; they turn it on \
             under Google Drive storage in the compute settings, or with `orx compute drive on`."
        )),
        None => Err(anyhow!(
            "No Google Drive folder is linked. Ask the user for the folder link, then run \
             `orx compute drive set <folder-link> --on` once they agree."
        )),
    }
}

/// A path inside the linked folder: `/`-separated, without `..`.
fn drive_path(path: &str) -> Result<String> {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.contains(&"..") {
        return Err(anyhow!(
            "'{path}' leaves the linked Drive folder. Use a path inside it, such as datasets/cifar."
        ));
    }
    Ok(parts.join("/"))
}

fn join(folder: &str, name: &str) -> String {
    [folder, name]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn file_name(path: &Path) -> Option<String> {
    match path.components().next_back()? {
        Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    }
}

/// `rclone copy` arguments that upload `local` into `folder`: a file lands at
/// `<folder>/<name>`, a directory at `<folder>/<dirname>/…`. Returns the Drive path written.
fn push_args(
    settings: &DriveSettings,
    local: &Path,
    is_dir: bool,
    folder: &str,
) -> Result<(Vec<String>, String)> {
    let folder = drive_path(folder)?;
    let dest = if is_dir {
        let name = file_name(local).ok_or_else(|| {
            anyhow!(
                "Name the directory to upload, such as ./data rather than {}.",
                local.display()
            )
        })?;
        join(&folder, &name)
    } else {
        folder.clone()
    };
    let written = match (is_dir, file_name(local)) {
        (false, Some(name)) => join(&folder, &name),
        _ => dest.clone(),
    };
    Ok((
        vec![
            "copy".into(),
            local.display().to_string(),
            settings.target(&dest),
        ],
        written,
    ))
}

async fn transfer(settings: &DriveSettings, args: &[String]) -> Result<()> {
    let mut command = base_command(settings)?;
    command.args(args).args([
        "--stats",
        "1m",
        "--stats-one-line",
        "--stats-log-level",
        "NOTICE",
    ]);
    let status = command.status().await.context("Could not run rclone")?;
    if status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "rclone {} stopped with {status}; see its message above. If the Google sign-in expired, \
         run `rclone config reconnect {}:` and try again.",
        args.first().map(String::as_str).unwrap_or_default(),
        settings.remote
    ))
}

/// Uploads a file or directory into the linked folder; returns the Drive path written.
pub async fn push(local: &Path, folder: &str) -> Result<String> {
    let settings = require_enabled()?;
    let meta =
        std::fs::metadata(local).map_err(|e| anyhow!("Could not read {}: {e}", local.display()))?;
    let (args, written) = push_args(&settings, local, meta.is_dir(), folder)?;
    transfer(&settings, &args).await?;
    Ok(written)
}

/// Downloads a file or folder from the linked folder into `into`; returns the local path.
pub async fn pull(path: &str, into: &Path) -> Result<PathBuf> {
    let settings = require_enabled()?;
    let path = drive_path(path)?;
    let entry = stat(&settings, &path).await?;
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    let local = if entry.is_dir && !name.is_empty() {
        into.join(&name)
    } else {
        into.to_path_buf()
    };
    let args = vec![
        "copy".into(),
        settings.target(&path),
        local.display().to_string(),
    ];
    transfer(&settings, &args).await?;
    Ok(if entry.is_dir {
        local
    } else {
        local.join(name)
    })
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Entry {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Size", default)]
    pub size: i64,
    #[serde(rename = "ModTime", default)]
    pub mod_time: String,
    #[serde(rename = "IsDir", default)]
    pub is_dir: bool,
}

async fn rclone_json(settings: &DriveSettings, args: &[&str], path: &str) -> Result<Vec<u8>> {
    let mut command = base_command(settings)?;
    command.args(args).arg(settings.target(path));
    let output = command.output().await.context("Could not run rclone")?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(3) || stderr.contains("not found") {
        return Err(anyhow!(
            "'{path}' is not in the linked Drive folder. List it with `orx compute drive ls`."
        ));
    }
    Err(anyhow!(
        "rclone could not read '{path}' in {}: {}",
        settings.folder_url,
        stderr.trim().lines().last().unwrap_or("no details")
    ))
}

async fn stat(settings: &DriveSettings, path: &str) -> Result<Entry> {
    let raw = rclone_json(settings, &["lsjson", "--stat"], path).await?;
    serde_json::from_slice(&raw).context("rclone returned an unreadable listing")
}

/// What `path` (a folder inside the linked one) holds.
pub async fn list(path: &str) -> Result<Vec<Entry>> {
    let settings = require_enabled()?;
    let path = drive_path(path)?;
    let raw = rclone_json(&settings, &["lsjson"], &path).await?;
    serde_json::from_slice(&raw).context("rclone returned an unreadable listing")
}

/// The settings as the CLI and the dashboard show them, with the Drive remotes found in the
/// rclone config and why the folder cannot be used now, if it cannot.
pub fn describe(settings: Option<&DriveSettings>) -> serde_json::Value {
    let config = settings
        .and_then(|s| s.rclone_config.clone())
        .or_else(default_rclone_config);
    let raw = config.as_deref().map(read_config);
    let remotes = match &raw {
        Some(Ok(raw)) => drive_remotes(raw),
        _ => Vec::new(),
    };
    let problem = if crate::local::shell_env::find("rclone").is_none() {
        Some(format!("rclone is not installed. {INSTALL_HINT}"))
    } else {
        match (&raw, settings) {
            (None, _) => Some(format!("No rclone config found. {INSTALL_HINT}")),
            (Some(Err(e)), _) => Some(e.to_string()),
            (Some(Ok(raw)), Some(settings)) => check_remote(
                raw,
                &settings.remote,
                config.as_deref().unwrap_or(Path::new("")),
            )
            .err()
            .map(|e| e.to_string()),
            (Some(Ok(_)), None) if remotes.is_empty() => Some(format!(
                "{} has no Google Drive remote. {INSTALL_HINT}",
                config
                    .as_deref()
                    .map(Path::display)
                    .map(|p| p.to_string())
                    .unwrap_or_default()
            )),
            _ => None,
        }
    };
    serde_json::json!({
        "enabled": settings.is_some_and(|s| s.enabled),
        "folderUrl": settings.map(|s| s.folder_url.as_str()),
        "folderId": settings.map(|s| s.folder_id.as_str()),
        "remote": settings.map(|s| s.remote.as_str()),
        "rcloneConfig": config.as_deref().map(Path::display).map(|p| p.to_string()),
        "remotes": remotes,
        "error": problem,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "[s3]\ntype = s3\n\n[gdrive]\ntype = drive\nscope = drive\ntoken = {}\n\n[narrow]\ntype = drive\nscope = drive.file\n\n[vault]\ntype = crypt\nremote = gdrive:vault\n";

    fn settings() -> DriveSettings {
        DriveSettings {
            enabled: true,
            folder_url: "https://drive.google.com/drive/folders/1XVF_efi99XHJERnu".into(),
            folder_id: "1XVF_efi99XHJERnu".into(),
            remote: "gdrive".into(),
            rclone_config: None,
        }
    }

    #[test]
    fn folder_links_give_their_id() {
        let id = "1XVF_efi99XHJERnu-eBKzW5Km0YiNUgc";
        for link in [
            format!("https://drive.google.com/drive/folders/{id}?usp=sharing"),
            format!("https://drive.google.com/drive/u/1/folders/{id}"),
            format!("https://drive.google.com/open?id={id}"),
            format!("  {id}  "),
        ] {
            assert_eq!(folder_id(&link).unwrap(), id, "{link}");
        }
        assert!(folder_id("https://drive.google.com/drive/my-drive").is_err());
        assert!(folder_id("gdrive:openresearch").is_err());
    }

    #[test]
    fn only_drive_remotes_that_can_write_shared_folders_are_used() {
        assert_eq!(drive_remotes(CONFIG), ["gdrive", "narrow"]);
        let conf = Path::new("rclone.conf");
        assert!(check_remote(CONFIG, "gdrive", conf).is_ok());
        let narrow = check_remote(CONFIG, "narrow", conf)
            .unwrap_err()
            .to_string();
        assert!(narrow.contains("scope"), "{narrow}");
        let s3 = check_remote(CONFIG, "s3", conf).unwrap_err().to_string();
        assert!(s3.contains("not Google Drive"), "{s3}");
        assert!(check_remote(CONFIG, "missing", conf).is_err());
    }

    #[test]
    fn paths_stay_inside_the_linked_folder() {
        assert_eq!(drive_path("/datasets//cifar/").unwrap(), "datasets/cifar");
        assert_eq!(drive_path("").unwrap(), "");
        assert!(drive_path("datasets/../../other").is_err());
        assert_eq!(
            settings().target("datasets"),
            "gdrive,root_folder_id=1XVF_efi99XHJERnu:datasets"
        );
    }

    #[test]
    fn a_push_keeps_the_file_or_directory_name() {
        let (args, written) =
            push_args(&settings(), Path::new("out/model.pt"), false, "checkpoints").unwrap();
        assert_eq!(
            args,
            [
                "copy",
                "out/model.pt",
                "gdrive,root_folder_id=1XVF_efi99XHJERnu:checkpoints"
            ]
        );
        assert_eq!(written, "checkpoints/model.pt");
        let (args, written) = push_args(&settings(), Path::new("./data/"), true, "").unwrap();
        assert_eq!(args[2], "gdrive,root_folder_id=1XVF_efi99XHJERnu:data");
        assert_eq!(written, "data");
    }

    #[test]
    fn storage_stays_off_until_turned_on_and_round_trips() {
        // Alone in a child, so no other test reads these settings.
        if !crate::config::in_test_child(&[]) {
            return;
        }
        let dir = crate::config::test_home().join("drive-storage");
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("rclone.conf");
        std::fs::write(
            &conf,
            CONFIG.replace("\n[narrow]\ntype = drive\nscope = drive.file\n", ""),
        )
        .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let link =
            "https://drive.google.com/drive/folders/1XVF_efi99XHJERnu-eBKzW5Km0YiNUgc?usp=sharing";

        assert!(require_enabled().is_err(), "nothing is linked yet");
        let saved = runtime
            .block_on(update(Update {
                folder: Some(link.into()),
                rclone_config: Some(conf.clone()),
                ..Update::default()
            }))
            .unwrap()
            .unwrap();
        assert!(!saved.enabled, "linking a folder leaves storage off");
        assert_eq!(saved.remote, "gdrive", "the only Drive remote is picked");
        assert_eq!(load_settings().unwrap(), Some(saved.clone()));
        let off = require_enabled().unwrap_err().to_string();
        assert!(off.contains("off"), "{off}");
        assert_eq!(describe(Some(&saved))["enabled"], false);
        assert_eq!(
            describe(Some(&saved))["folderId"],
            "1XVF_efi99XHJERnu-eBKzW5Km0YiNUgc"
        );

        let bad = runtime.block_on(update(Update {
            remote: Some("s3".into()),
            ..Update::default()
        }));
        assert!(bad.is_err());
        assert_eq!(
            load_settings().unwrap(),
            Some(saved),
            "a rejected change saves nothing"
        );

        runtime
            .block_on(update(Update {
                folder: Some(String::new()),
                ..Update::default()
            }))
            .unwrap();
        assert_eq!(load_settings().unwrap(), None);
    }
}
