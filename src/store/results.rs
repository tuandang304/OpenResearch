//! A project's results settings: which baseline its runs are compared with,
//! which way each metric improves, and the metric that ranks runs. Kept in its
//! own versioned table beside the run measurements in `run_metrics`.
use std::collections::BTreeMap;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::run_metrics::RunMetrics;
use super::Store;
use crate::error::{anyhow, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Goal {
    /// Higher is better (accuracy, F1).
    Max,
    /// Lower is better (loss, error rate).
    Min,
}

/// Baseline values entered without a run, such as numbers from a paper.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualBaseline {
    pub name: String,
    pub measurement: RunMetrics,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResultsSettings {
    /// A run pinned as the baseline. Absent with no `manual_baseline`, the
    /// newest measured run of the root experiment is the baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_baseline: Option<ManualBaseline>,
    /// Overrides of the direction guessed from a metric's name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub goals: BTreeMap<String, Goal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_metric: Option<String>,
    /// Fields a newer version wrote, kept when this one rewrites the record.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ResultsSettings {
    pub fn validate(&self) -> Result<()> {
        if self.baseline_run_id.is_some() && self.manual_baseline.is_some() {
            return Err(anyhow!(
                "Choose either a baseline run or baseline values, not both."
            ));
        }
        if let Some(manual) = &self.manual_baseline {
            if manual.name.trim().is_empty() || manual.name.len() > 256 {
                return Err(anyhow!("Give the baseline a name of 1–256 characters."));
            }
            manual.measurement.validate()?;
        }
        let valid = |name: &str| !name.trim().is_empty() && name.len() <= 256;
        if self.goals.len() > 128 || !self.goals.keys().all(|name| valid(name)) {
            return Err(anyhow!("Give 1–128 named metric directions."));
        }
        if self
            .primary_metric
            .as_deref()
            .is_some_and(|name| !valid(name))
        {
            return Err(anyhow!("The primary metric needs a name."));
        }
        Ok(())
    }
}

impl Store {
    fn ensure_results_settings(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS orx_results_settings_v1 (
                project_id TEXT PRIMARY KEY NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS orx_results_settings_v1_cleanup
            AFTER DELETE ON local_projects BEGIN
                DELETE FROM orx_results_settings_v1 WHERE project_id = OLD.id;
            END;",
        )?;
        Ok(())
    }

    pub fn results_settings(&self, project_id: &str) -> Result<ResultsSettings> {
        self.ensure_results_settings()?;
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM orx_results_settings_v1 WHERE project_id = ?1",
                [project_id],
                |row| row.get(0),
            )
            .optional()?;
        match payload {
            Some(payload) => serde_json::from_str(&payload).map_err(|error| {
                anyhow!("The results settings of project {project_id} are unreadable: {error}")
            }),
            None => Ok(ResultsSettings::default()),
        }
    }

    pub fn put_results_settings(&self, project_id: &str, settings: &ResultsSettings) -> Result<()> {
        settings.validate()?;
        self.ensure_results_settings()?;
        let tx = self.conn.unchecked_transaction()?;
        if let Some(run_id) = &settings.baseline_run_id {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT project_id FROM runs WHERE id = ?1",
                    [run_id],
                    |row| row.get(0),
                )
                .optional()?;
            match owner {
                None => return Err(anyhow!("Run {run_id} was not found.")),
                Some(owner) if owner != project_id => {
                    return Err(anyhow!("Run {run_id} belongs to another project."))
                }
                Some(_) => {}
            }
        }
        tx.execute(
            "INSERT INTO orx_results_settings_v1(project_id, payload) VALUES (?1, ?2)
             ON CONFLICT(project_id) DO UPDATE SET payload = excluded.payload",
            rusqlite::params![project_id, serde_json::to_string(settings)?],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_round_trip_and_keep_fields_a_newer_version_adds() {
        let root = std::env::temp_dir().join(format!("orx-results-{}", uuid::Uuid::new_v4()));
        {
            let store = Store::open_at(root.clone()).unwrap();
            assert_eq!(
                store.results_settings("p1").unwrap(),
                ResultsSettings::default()
            );
            store.conn.execute("INSERT INTO runs (id, experiment_id, project_id, status, backend_json, created_at, updated_at) VALUES ('r1', 'e1', 'p1', 'done', '{}', 0, 0)", []).unwrap();
            let pinned = ResultsSettings {
                baseline_run_id: Some("r1".into()),
                goals: BTreeMap::from([("loss".into(), Goal::Min)]),
                primary_metric: Some("loss".into()),
                ..Default::default()
            };
            store.put_results_settings("p1", &pinned).unwrap();
            assert_eq!(store.results_settings("p1").unwrap(), pinned);
            assert!(store.put_results_settings("p2", &pinned).is_err());
            let missing = ResultsSettings {
                baseline_run_id: Some("nope".into()),
                ..Default::default()
            };
            assert!(store.put_results_settings("p1", &missing).is_err());

            let newer: ResultsSettings = serde_json::from_value(json!({
                "baselineRunId": "r1", "goals": {"acc": "max"}, "addedLater": true
            }))
            .unwrap();
            assert_eq!(newer.goals["acc"], Goal::Max);
            store.put_results_settings("p1", &newer).unwrap();
            assert_eq!(
                store.results_settings("p1").unwrap().extra["addedLater"],
                json!(true)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_ambiguous_or_invalid_baselines() {
        let manual: ManualBaseline = serde_json::from_value(json!({
            "name": "Paper",
            "measurement": {"version": 1, "dataset": "cifar10", "split": "test", "metrics": {"acc": 0.9}, "seed": null}
        }))
        .unwrap();
        let both = ResultsSettings {
            baseline_run_id: Some("r1".into()),
            manual_baseline: Some(manual.clone()),
            ..Default::default()
        };
        assert!(both.validate().is_err());
        let mut unnamed = manual;
        unnamed.name = " ".into();
        let settings = ResultsSettings {
            manual_baseline: Some(unnamed),
            ..Default::default()
        };
        assert!(settings.validate().is_err());
    }
}
