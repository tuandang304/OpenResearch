//! `orx compute drive`, owned by the tuandang304 fork: the user's opt-in Google Drive folder
//! for datasets, checkpoints and other large outputs. See `jobs/drive_storage.rs`.

use super::*;
use crate::jobs::drive_storage::{self as drive, Update};

#[derive(Debug, Subcommand)]
pub enum DriveCommand {
    /// Show the linked folder, whether storage is on, and what stops it from working.
    Show,
    /// Link a Google Drive folder (its share link). Storage stays off unless --on is given.
    Set {
        folder: String,
        /// The rclone Google Drive remote to use; the only one in the config otherwise.
        #[arg(long)]
        remote: Option<String>,
        /// The rclone config file holding the remote; rclone's default otherwise.
        #[arg(long)]
        rclone_config: Option<std::path::PathBuf>,
        /// Also turn storage on (only when the user asked for it).
        #[arg(long)]
        on: bool,
    },
    /// Turn storage on; checks that the folder opens first.
    On,
    /// Turn storage off; files already in Drive stay there.
    Off,
    /// Forget the linked folder.
    Clear,
    /// List a folder inside the linked one (its top level by default).
    Ls {
        #[arg(default_value = "")]
        path: String,
    },
    /// Upload a file or directory into the linked folder, keeping its name.
    Push {
        local: std::path::PathBuf,
        /// Folder inside the linked one, such as datasets or checkpoints/run-3.
        #[arg(long, default_value = "")]
        to: String,
    },
    /// Download a file or folder from the linked folder.
    Pull {
        path: String,
        /// Local directory to download into (the current one by default).
        #[arg(long, default_value = ".")]
        to: std::path::PathBuf,
    },
}

pub(super) async fn run(command: DriveCommand, json: bool) -> Result<()> {
    let change = |enabled: Option<bool>| Update {
        enabled,
        ..Update::default()
    };
    match command {
        DriveCommand::Show => {}
        DriveCommand::Set {
            folder,
            remote,
            rclone_config,
            on,
        } => {
            drive::update(Update {
                enabled: on.then_some(true),
                folder: Some(folder),
                remote,
                rclone_config,
            })
            .await?;
        }
        DriveCommand::On => {
            drive::update(change(Some(true))).await?;
        }
        DriveCommand::Off => {
            drive::update(change(Some(false))).await?;
        }
        DriveCommand::Clear => drive::clear_settings()?,
        DriveCommand::Ls { path } => {
            let entries = drive::list(&path).await?;
            if json {
                println!("{}", serde_json::to_string(&entries)?);
            } else if entries.is_empty() {
                println!("Empty.");
            } else {
                let rows: Vec<Vec<String>> = entries
                    .iter()
                    .map(|entry| {
                        vec![
                            if entry.is_dir {
                                format!("{}/", entry.path)
                            } else {
                                entry.path.clone()
                            },
                            if entry.is_dir {
                                String::new()
                            } else {
                                entry.size.to_string()
                            },
                            entry.mod_time.chars().take(19).collect(),
                        ]
                    })
                    .collect();
                print_table(&["NAME", "BYTES", "MODIFIED"], &rows);
            }
            return Ok(());
        }
        DriveCommand::Push { local, to } => {
            let written = drive::push(&local, &to).await?;
            return report(
                json,
                json!({"saved": written}),
                &format!("Saved to Drive: {written}"),
            );
        }
        DriveCommand::Pull { path, to } => {
            let local = drive::pull(&path, &to).await?;
            let shown = local.display().to_string();
            return report(
                json,
                json!({"downloaded": shown}),
                &format!("Downloaded to {shown}"),
            );
        }
    }
    let value = drive::describe(drive::load_settings()?.as_ref());
    if json {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

fn report(json: bool, value: Value, text: &str) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{text}");
    }
    Ok(())
}
