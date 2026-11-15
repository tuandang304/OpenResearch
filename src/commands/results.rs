//! `orx results`: a project's measured runs ranked against its baseline, the
//! same view as the Results tab, and the settings that define the comparison.
use std::io::Read;

use crate::error::{anyhow, Context, Result};
use crate::local::results;
use crate::store::results::{Goal, ManualBaseline};
use crate::store::run_metrics::{RunMetrics, MAX_BYTES};
use crate::store::Store;

#[derive(Debug, clap::Args)]
pub struct ResultsArgs {
    #[command(subcommand)]
    pub command: ResultsCommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum ResultsCommand {
    /// Rank the project's measured runs against its baseline, with warnings.
    View {
        project_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Choose the baseline, the ranking metric, or which way a metric improves.
    Set(SetArgs),
}

#[derive(Debug, clap::Args)]
pub struct SetArgs {
    pub project_id: String,
    /// Pin this run as the baseline.
    #[arg(long, conflicts_with_all = ["baseline_file", "baseline_root"])]
    pub baseline_run: Option<String>,
    /// Use values measured elsewhere (a version-1 metrics JSON file, - for stdin) as the baseline.
    #[arg(long, conflicts_with = "baseline_root", requires = "baseline_name")]
    pub baseline_file: Option<String>,
    /// Name shown for a baseline given with --baseline-file, such as the paper or model.
    #[arg(long)]
    pub baseline_name: Option<String>,
    /// Go back to the newest measured run of the root experiment as the baseline.
    #[arg(long)]
    pub baseline_root: bool,
    /// The metric that ranks runs and decides better or worse.
    #[arg(long)]
    pub primary: Option<String>,
    /// A metric where lower is better. Repeatable.
    #[arg(long = "lower-is-better", value_name = "METRIC")]
    pub lower: Vec<String>,
    /// A metric where higher is better. Repeatable.
    #[arg(long = "higher-is-better", value_name = "METRIC")]
    pub higher: Vec<String>,
}

pub fn run(args: ResultsArgs) -> Result<()> {
    let store = Store::open()?;
    match args.command {
        ResultsCommand::View { project_id, json } => view(&store, &project_id, json),
        ResultsCommand::Set(args) => set(&store, args),
    }
}

fn view(store: &Store, project_id: &str, json: bool) -> Result<()> {
    crate::local::resolve::resolve_project(store, project_id)?;
    let report = results::report(store, project_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", results::render(&report));
    }
    Ok(())
}

fn set(store: &Store, args: SetArgs) -> Result<()> {
    let project = crate::local::resolve::resolve_project(store, &args.project_id)?;
    let mut settings = store.results_settings(&project.id)?;
    if let Some(run) = args.baseline_run {
        settings.baseline_run_id = Some(run);
        settings.manual_baseline = None;
    }
    if let Some(path) = args.baseline_file {
        let input: Box<dyn Read> = if path == "-" {
            Box::new(std::io::stdin())
        } else {
            Box::new(std::fs::File::open(&path).with_context(|| format!("Could not open {path}"))?)
        };
        let mut bytes = Vec::new();
        input.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(anyhow!("Metrics exceed the 64 KiB limit."));
        }
        let measurement: RunMetrics = serde_json::from_slice(&bytes)
            .with_context(|| format!("{path} is not version-1 metrics JSON"))?;
        settings.manual_baseline = Some(ManualBaseline {
            name: args.baseline_name.unwrap_or_default(),
            measurement,
        });
        settings.baseline_run_id = None;
    }
    if args.baseline_root {
        settings.baseline_run_id = None;
        settings.manual_baseline = None;
    }
    if let Some(primary) = args.primary {
        settings.primary_metric = Some(primary);
    }
    for (metrics, goal) in [(args.lower, Goal::Min), (args.higher, Goal::Max)] {
        for metric in metrics {
            settings.goals.insert(metric, goal);
        }
    }
    store.put_results_settings(&project.id, &settings)?;
    view(store, &project.id, false)
}
