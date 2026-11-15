//! A project's results compared with its baseline: the ranking, verdicts and
//! warnings the Results tab shows and `orx results view` prints. Computed from
//! the run measurements agents record with `orx exp metrics`.
use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::error::Result;
use crate::store::results::{Goal, ResultsSettings};
use crate::store::run_metrics::RunMetrics;
use crate::store::{Store, StoredRun};

/// Name fragments of metrics where lower is better.
const LOWER_IS_BETTER: &[&str] = &[
    "loss",
    "error",
    "err",
    "perplexity",
    "ppl",
    "wer",
    "cer",
    "mse",
    "mae",
    "rmse",
    "nll",
    "bpb",
    "bpc",
    "fid",
    "latency",
    "time",
    "cost",
    "regret",
];

/// Name fragments that make a metric the default ranking metric, in order.
const PRIMARY_HINTS: &[&str] = &[
    "accuracy", "acc", "f1", "auc", "map", "bleu", "rouge", "reward", "score", "loss",
];

pub fn guess_goal(metric: &str) -> Goal {
    let name = metric.to_lowercase();
    if LOWER_IS_BETTER.iter().any(|hint| name.contains(hint)) {
        Goal::Min
    } else {
        Goal::Max
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BaselineSource {
    /// A run the user or agent pinned.
    Pinned,
    /// The newest measured run of the root experiment, used until one is pinned.
    Root,
    /// Values entered without a run.
    Manual,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub source: BaselineSource,
    pub name: String,
    pub run_id: Option<String>,
    pub experiment_id: Option<String>,
    /// `None` while a pinned run has no measurements yet.
    pub measurement: Option<RunMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Baseline,
    Better,
    Worse,
    Same,
    /// Measured on another dataset or split than the baseline.
    Incomparable,
    /// No baseline value, or no value for the primary metric.
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub run_id: String,
    pub experiment_id: String,
    pub experiment_title: String,
    pub status: String,
    pub commit_sha: Option<String>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    pub measurement: RunMetrics,
    /// Value minus the baseline's, for each metric both have on the same data.
    pub deltas: BTreeMap<String, f64>,
    pub verdict: Verdict,
    /// 1-based position by the primary metric; `None` when it cannot be ranked.
    pub rank: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unmeasured {
    pub run_id: String,
    pub experiment_id: String,
    pub experiment_title: String,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Warning {
    /// The newest measured run is worse than the baseline on the primary metric.
    #[serde(rename_all = "camelCase")]
    WorseThanBaseline {
        run_id: String,
        experiment_title: String,
        metric: String,
        value: f64,
        baseline: f64,
        delta: f64,
    },
    /// The newest measured run used other data than the baseline.
    #[serde(rename_all = "camelCase")]
    Incomparable {
        run_id: String,
        experiment_title: String,
        dataset: String,
        split: String,
    },
    /// Finished runs no one recorded measurements for.
    Unmeasured { count: usize },
    /// The pinned baseline run has no measurements.
    #[serde(rename_all = "camelCase")]
    BaselineUnmeasured { run_id: String },
    /// Runs are measured but nothing to compare them with.
    NoBaseline,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub project_id: String,
    pub settings: ResultsSettings,
    pub baseline: Option<Baseline>,
    pub primary_metric: Option<String>,
    /// Every metric seen, with the direction that counts as better.
    pub goals: BTreeMap<String, Goal>,
    /// Measured runs, ranked first, then the unranked newest first.
    pub rows: Vec<Row>,
    /// Finished runs without measurements, newest first.
    pub unmeasured: Vec<Unmeasured>,
    pub warnings: Vec<Warning>,
}

pub fn report(store: &Store, project_id: &str) -> Result<Report> {
    let settings = store.results_settings(project_id)?;
    let measurements = store.project_run_metrics(project_id)?;
    let runs = store.list_runs_by_project(project_id)?;
    let experiments = store.list_experiments_by_project(project_id)?;
    Ok(build(
        project_id,
        settings,
        &runs,
        &experiments
            .iter()
            .map(|experiment| Experiment {
                id: &experiment.id,
                title: experiment.title.as_deref().unwrap_or(&experiment.slug),
                root: experiment.parent_experiment_id.is_none(),
                archived: experiment.archived,
            })
            .collect::<Vec<_>>(),
        measurements,
    ))
}

struct Experiment<'a> {
    id: &'a str,
    title: &'a str,
    root: bool,
    archived: bool,
}

fn build(
    project_id: &str,
    settings: ResultsSettings,
    runs: &[StoredRun],
    experiments: &[Experiment<'_>],
    mut measurements: BTreeMap<String, RunMetrics>,
) -> Report {
    let experiment = |id: &str| experiments.iter().find(|experiment| experiment.id == id);
    let title = |id: &str| experiment(id).map_or_else(|| id.to_string(), |e| e.title.to_string());
    // Newest first, so the first match is the newest.
    let mut runs: Vec<&StoredRun> = runs.iter().collect();
    runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));

    let baseline = if let Some(manual) = &settings.manual_baseline {
        Some(Baseline {
            source: BaselineSource::Manual,
            name: manual.name.clone(),
            run_id: None,
            experiment_id: None,
            measurement: Some(manual.measurement.clone()),
        })
    } else {
        let pinned = settings.baseline_run_id.as_deref();
        let run = match pinned {
            Some(id) => runs.iter().find(|run| run.id == id),
            None => runs.iter().find(|run| {
                measurements.contains_key(&run.id)
                    && experiment(&run.experiment_id).is_some_and(|e| e.root && !e.archived)
            }),
        };
        run.map(|run| Baseline {
            source: if pinned.is_some() {
                BaselineSource::Pinned
            } else {
                BaselineSource::Root
            },
            name: title(&run.experiment_id),
            run_id: Some(run.id.clone()),
            experiment_id: Some(run.experiment_id.clone()),
            measurement: measurements.get(&run.id).cloned(),
        })
    };
    let reference = baseline.as_ref().and_then(|b| b.measurement.as_ref());

    let mut names: Vec<&String> = measurements
        .values()
        .flat_map(|m| m.metrics.keys())
        .collect();
    names.extend(reference.iter().flat_map(|m| m.metrics.keys()));
    let goals: BTreeMap<String, Goal> = names
        .into_iter()
        .map(|name| {
            let goal = settings
                .goals
                .get(name)
                .copied()
                .unwrap_or_else(|| guess_goal(name));
            (name.clone(), goal)
        })
        .collect();
    let primary = settings.primary_metric.clone().or_else(|| {
        let candidates: Vec<&String> = match reference {
            Some(m) if !m.metrics.is_empty() => m.metrics.keys().collect(),
            _ => goals.keys().collect(),
        };
        PRIMARY_HINTS
            .iter()
            .find_map(|hint| {
                candidates
                    .iter()
                    .find(|name| name.to_lowercase().contains(hint))
            })
            .or(candidates.first())
            .map(|name| (*name).clone())
    });

    let baseline_run = baseline.as_ref().and_then(|b| b.run_id.as_deref());
    let mut rows = Vec::new();
    let mut unmeasured = Vec::new();
    for run in &runs {
        let is_baseline = baseline_run == Some(run.id.as_str());
        let hidden = experiment(&run.experiment_id).is_none_or(|e| e.archived);
        if hidden && !is_baseline {
            continue;
        }
        let Some(measurement) = measurements.remove(&run.id) else {
            if run.status == "done" && !is_baseline {
                unmeasured.push(Unmeasured {
                    run_id: run.id.clone(),
                    experiment_id: run.experiment_id.clone(),
                    experiment_title: title(&run.experiment_id),
                    ended_at: run.ended_at,
                });
            }
            continue;
        };
        let same_data = reference
            .is_some_and(|r| r.dataset == measurement.dataset && r.split == measurement.split);
        let deltas = match reference {
            Some(r) if same_data => measurement
                .metrics
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), value - r.metrics.get(name)?)))
                .filter(|(_, delta)| delta.is_finite())
                .collect(),
            _ => BTreeMap::new(),
        };
        let verdict = if is_baseline {
            Verdict::Baseline
        } else if reference.is_some() && !same_data {
            Verdict::Incomparable
        } else {
            match primary.as_ref().and_then(|p| Some((p, deltas.get(p)?))) {
                None => Verdict::Unknown,
                Some((metric, delta)) => {
                    let gain = match goals.get(metric) {
                        Some(Goal::Min) => -delta,
                        _ => *delta,
                    };
                    if gain > 0.0 {
                        Verdict::Better
                    } else if gain < 0.0 {
                        Verdict::Worse
                    } else {
                        Verdict::Same
                    }
                }
            }
        };
        rows.push(Row {
            run_id: run.id.clone(),
            experiment_id: run.experiment_id.clone(),
            experiment_title: title(&run.experiment_id),
            status: run.status.clone(),
            commit_sha: run.commit_sha.clone(),
            created_at: run.created_at,
            ended_at: run.ended_at,
            measurement,
            deltas,
            verdict,
            rank: None,
        });
    }

    let warnings = warnings(&rows, &unmeasured, baseline.as_ref(), primary.as_deref());
    rank(&mut rows, primary.as_deref(), &goals);
    Report {
        project_id: project_id.to_string(),
        settings,
        baseline,
        primary_metric: primary,
        goals,
        rows,
        unmeasured,
        warnings,
    }
}

/// `rows` arrive newest first.
fn warnings(
    rows: &[Row],
    unmeasured: &[Unmeasured],
    baseline: Option<&Baseline>,
    primary: Option<&str>,
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    match baseline {
        Some(Baseline {
            measurement: None,
            run_id: Some(run_id),
            ..
        }) => warnings.push(Warning::BaselineUnmeasured {
            run_id: run_id.clone(),
        }),
        None if !rows.is_empty() => warnings.push(Warning::NoBaseline),
        _ => {}
    }
    if let (Some(latest), Some(reference)) = (
        rows.iter().find(|row| row.verdict != Verdict::Baseline),
        baseline.and_then(|b| b.measurement.as_ref()),
    ) {
        match latest.verdict {
            Verdict::Worse => {
                let metric = primary.unwrap_or_default();
                warnings.push(Warning::WorseThanBaseline {
                    run_id: latest.run_id.clone(),
                    experiment_title: latest.experiment_title.clone(),
                    metric: metric.to_string(),
                    value: latest.measurement.metrics[metric],
                    baseline: reference.metrics[metric],
                    delta: latest.deltas[metric],
                });
            }
            Verdict::Incomparable => warnings.push(Warning::Incomparable {
                run_id: latest.run_id.clone(),
                experiment_title: latest.experiment_title.clone(),
                dataset: latest.measurement.dataset.clone(),
                split: latest.measurement.split.clone(),
            }),
            _ => {}
        }
    }
    if !unmeasured.is_empty() {
        warnings.push(Warning::Unmeasured {
            count: unmeasured.len(),
        });
    }
    warnings
}

fn rank(rows: &mut [Row], primary: Option<&str>, goals: &BTreeMap<String, Goal>) {
    let Some(primary) = primary else { return };
    let value = |row: &Row| {
        row.measurement
            .metrics
            .get(primary)
            .copied()
            .filter(|_| row.verdict != Verdict::Incomparable)
    };
    let lower = goals.get(primary) == Some(&Goal::Min);
    // Stable: ties and unranked rows keep newest first.
    rows.sort_by(|a, b| match (value(a), value(b)) {
        (Some(x), Some(y)) if lower => x.total_cmp(&y),
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let mut position = 0;
    for row in rows.iter_mut() {
        if value(row).is_some() {
            position += 1;
            row.rank = Some(position);
        }
    }
}

fn number(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

fn signed(value: f64) -> String {
    if value > 0.0 {
        format!("+{}", number(value))
    } else {
        number(value)
    }
}

/// One line comparing a run with the baseline, for agents.
pub fn run_comparison(report: &Report, run_id: &str) -> Option<String> {
    let row = report.rows.iter().find(|row| row.run_id == run_id)?;
    let baseline = report.baseline.as_ref()?;
    let metric = report.primary_metric.as_deref()?;
    let reference = baseline.measurement.as_ref()?;
    Some(match row.verdict {
        Verdict::Baseline => format!("Run {run_id} is the baseline ({}).", baseline.name),
        Verdict::Incomparable => format!(
            "Run {run_id} is not comparable with baseline \"{}\": it was measured on {}/{}, the baseline on {}/{}.",
            baseline.name, row.measurement.dataset, row.measurement.split, reference.dataset, reference.split
        ),
        Verdict::Unknown => format!(
            "Run {run_id} has no {metric} to compare with baseline \"{}\".",
            baseline.name
        ),
        verdict => format!(
            "{}Run {run_id}: {metric} {} vs baseline \"{}\" {} ({}, {}).",
            if verdict == Verdict::Worse { "Warning: " } else { "" },
            number(row.measurement.metrics[metric]),
            baseline.name,
            number(reference.metrics[metric]),
            signed(row.deltas[metric]),
            match verdict {
                Verdict::Better => "better",
                Verdict::Worse => "worse",
                _ => "same",
            },
        ),
    })
}

pub fn render(report: &Report) -> String {
    let mut out = String::new();
    match &report.baseline {
        Some(baseline) => {
            let source = match baseline.source {
                BaselineSource::Pinned => "pinned run",
                BaselineSource::Root => "newest measured run of the root experiment",
                BaselineSource::Manual => "entered values",
            };
            let _ = write!(out, "Baseline: {} ({source}", baseline.name);
            if let Some(run) = &baseline.run_id {
                let _ = write!(out, ", run {run}");
            }
            out.push(')');
            if let Some(m) = &baseline.measurement {
                let values: Vec<String> = m.metrics.iter().map(|(k, v)| format!("{k}={}", number(*v))).collect();
                let _ = write!(out, " on {}/{}: {}", m.dataset, m.split, values.join(", "));
            }
            out.push('\n');
        }
        None => out.push_str(
            "Baseline: none. Record measurements for a run of the root experiment, or pin one with `orx results set <projectId> --baseline-run <runId>`.\n",
        ),
    }
    if let Some(primary) = &report.primary_metric {
        let goal = match report.goals.get(primary) {
            Some(Goal::Min) => "lower is better",
            _ => "higher is better",
        };
        let _ = writeln!(out, "Ranked by: {primary} ({goal})");
    }
    for warning in &report.warnings {
        let text = match warning {
            Warning::WorseThanBaseline { run_id, metric, value, baseline, delta, .. } => format!(
                "Run {run_id} is worse than the baseline: {metric} {} vs {} ({}).",
                number(*value), number(*baseline), signed(*delta)
            ),
            Warning::Incomparable { run_id, dataset, split, .. } => format!(
                "Run {run_id} was measured on {dataset}/{split}, which the baseline was not, so it is not compared."
            ),
            Warning::Unmeasured { count } => format!(
                "{count} finished run(s) have no recorded measurements. Record each with `orx exp metrics <experimentId> --run <runId> --file metrics.json`."
            ),
            Warning::BaselineUnmeasured { run_id } => format!(
                "The baseline run {run_id} has no recorded measurements. Record them with `orx exp metrics <experimentId> --run {run_id} --file metrics.json`."
            ),
            Warning::NoBaseline => "No baseline to compare with.".into(),
        };
        let _ = writeln!(out, "Warning: {text}");
    }
    if report.rows.is_empty() {
        out.push_str("No measured runs yet.\n");
    }
    for row in &report.rows {
        let rank = row.rank.map_or_else(|| "-".into(), |rank| rank.to_string());
        let values: Vec<String> = row
            .measurement
            .metrics
            .iter()
            .map(|(name, value)| match row.deltas.get(name) {
                Some(delta) if row.verdict != Verdict::Baseline => {
                    format!("{name}={} ({})", number(*value), signed(*delta))
                }
                _ => format!("{name}={}", number(*value)),
            })
            .collect();
        let verdict = match row.verdict {
            Verdict::Baseline => "baseline",
            Verdict::Better => "better",
            Verdict::Worse => "worse",
            Verdict::Same => "same",
            Verdict::Incomparable => "other data",
            Verdict::Unknown => "not compared",
        };
        let _ = writeln!(
            out,
            "{rank:>3}. {} · run {} · {verdict} · {}",
            row.experiment_title,
            row.run_id,
            values.join(", ")
        );
    }
    for run in &report.unmeasured {
        let _ = writeln!(
            out,
            "  -  {} · run {} · finished, no measurements",
            run.experiment_title, run.run_id
        );
    }
    out
}

/// What a finished run's notice adds so the agent records its measurements.
pub fn notice_hint(store: &Store, run: &StoredRun) -> Result<Option<String>> {
    if run.status != "done" || store.get_run_metrics(&run.id)?.is_some() {
        return Ok(None);
    }
    Ok(Some(format!(
        "Record this run's final measurements for the Results tab: `orx exp metrics {} --run {} --file metrics.json` (format in `orx skill orx-results`).",
        run.experiment_id, run.id
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn run(id: &str, experiment: &str, created_at: i64, status: &str) -> StoredRun {
        StoredRun {
            id: id.into(),
            experiment_id: experiment.into(),
            project_id: "p".into(),
            status: status.into(),
            backend_json: "{}".into(),
            command: String::new(),
            created_at,
            updated_at: created_at,
            ended_at: Some(created_at + 1),
            exit_code: Some(0),
            commit_sha: None,
            result_markdown: None,
            cancel_requested: false,
            cancel_reason: None,
            chat_session_id: None,
        }
    }

    fn measured(metrics: serde_json::Value, split: &str) -> RunMetrics {
        serde_json::from_value(json!({"version": 1, "dataset": "cifar10", "split": split, "metrics": metrics, "seed": null})).unwrap()
    }

    fn experiments() -> Vec<Experiment<'static>> {
        vec![
            Experiment {
                id: "root",
                title: "Baseline CNN",
                root: true,
                archived: false,
            },
            Experiment {
                id: "wide",
                title: "Wider layers",
                root: false,
                archived: false,
            },
            Experiment {
                id: "old",
                title: "Archived",
                root: false,
                archived: true,
            },
        ]
    }

    #[test]
    fn ranks_against_the_root_baseline_and_warns_when_the_newest_run_is_worse() {
        let runs = [
            run("r0", "root", 1, "done"),
            run("r1", "wide", 2, "done"),
            run("r2", "wide", 3, "done"),
            run("r3", "wide", 4, "done"),
            run("r4", "old", 5, "done"),
        ];
        let measurements = BTreeMap::from([
            (
                "r0".to_string(),
                measured(json!({"accuracy": 0.80, "loss": 0.5}), "test"),
            ),
            (
                "r1".to_string(),
                measured(json!({"accuracy": 0.85, "loss": 0.4}), "test"),
            ),
            (
                "r2".to_string(),
                measured(json!({"accuracy": 0.75, "loss": 0.6}), "test"),
            ),
            (
                "r4".to_string(),
                measured(json!({"accuracy": 0.99}), "test"),
            ),
        ]);
        let report = build(
            "p",
            ResultsSettings::default(),
            &runs,
            &experiments(),
            measurements,
        );
        let baseline = report.baseline.as_ref().unwrap();
        assert_eq!(
            (baseline.source, baseline.run_id.as_deref()),
            (BaselineSource::Root, Some("r0"))
        );
        assert_eq!(report.primary_metric.as_deref(), Some("accuracy"));
        assert_eq!(report.goals["loss"], Goal::Min);
        let order: Vec<_> = report
            .rows
            .iter()
            .map(|row| (row.run_id.as_str(), row.rank, row.verdict))
            .collect();
        assert_eq!(
            order,
            [
                ("r1", Some(1), Verdict::Better),
                ("r0", Some(2), Verdict::Baseline),
                ("r2", Some(3), Verdict::Worse),
            ]
        );
        assert!((report.rows[0].deltas["loss"] + 0.1).abs() < 1e-9);
        assert_eq!(
            report
                .unmeasured
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["r3"]
        );
        assert!(matches!(&report.warnings[..], [
            Warning::WorseThanBaseline { run_id, metric, .. },
            Warning::Unmeasured { count: 1 },
        ] if run_id == "r2" && metric == "accuracy"));
        let line = run_comparison(&report, "r2").unwrap();
        assert!(
            line.starts_with("Warning:") && line.contains("-0.05") && line.contains("worse"),
            "{line}"
        );
        assert!(render(&report).contains("1. Wider layers · run r1 · better"));
    }

    #[test]
    fn settings_choose_the_baseline_direction_and_ranking_metric() {
        let runs = [
            run("r0", "root", 1, "done"),
            run("r1", "wide", 2, "done"),
            run("r2", "wide", 3, "done"),
        ];
        let measurements = BTreeMap::from([
            (
                "r0".to_string(),
                measured(json!({"accuracy": 0.80, "score": 10.0}), "test"),
            ),
            (
                "r1".to_string(),
                measured(json!({"accuracy": 0.70, "score": 8.0}), "test"),
            ),
            ("r2".to_string(), measured(json!({"accuracy": 0.90}), "val")),
        ]);
        let settings = ResultsSettings {
            baseline_run_id: Some("r1".into()),
            goals: BTreeMap::from([("score".into(), Goal::Min)]),
            primary_metric: Some("score".into()),
            ..Default::default()
        };
        let report = build("p", settings, &runs, &experiments(), measurements);
        assert_eq!(
            report.baseline.as_ref().unwrap().source,
            BaselineSource::Pinned
        );
        let verdicts: HashMap<_, _> = report
            .rows
            .iter()
            .map(|row| (row.run_id.as_str(), (row.verdict, row.rank)))
            .collect();
        assert_eq!(verdicts["r0"], (Verdict::Worse, Some(2)));
        assert_eq!(verdicts["r1"], (Verdict::Baseline, Some(1)));
        assert_eq!(verdicts["r2"], (Verdict::Incomparable, None));
        assert!(
            matches!(&report.warnings[..], [Warning::Incomparable { run_id, .. }] if run_id == "r2")
        );
    }

    #[test]
    fn missing_baselines_are_reported_not_guessed() {
        let runs = [run("r0", "root", 1, "done"), run("r1", "wide", 2, "done")];
        let measurements =
            BTreeMap::from([("r1".to_string(), measured(json!({"loss": 0.3}), "test"))]);
        let report = build(
            "p",
            ResultsSettings::default(),
            &runs,
            &experiments(),
            measurements.clone(),
        );
        assert!(report.baseline.is_none());
        assert_eq!(report.rows[0].verdict, Verdict::Unknown);
        assert!(matches!(
            &report.warnings[..],
            [Warning::NoBaseline, Warning::Unmeasured { count: 1 }]
        ));

        let pinned = ResultsSettings {
            baseline_run_id: Some("r0".into()),
            ..Default::default()
        };
        let report = build("p", pinned, &runs, &experiments(), measurements);
        assert!(
            matches!(&report.warnings[..], [Warning::BaselineUnmeasured { run_id }] if run_id == "r0")
        );
        assert!(report.unmeasured.is_empty());
    }
}
