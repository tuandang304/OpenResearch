// Separate test crate; it runs the built orx binary directly.
#![allow(clippy::disallowed_methods)]
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Output};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("orx-research-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orx"))
            .env("ORX_DATA_DIR", self.0.join("data"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("ORX_NO_UPDATE_CHECK", "1")
            .arg("--no-telemetry")
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn doctor_local_reports_the_executable_and_validates_options() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["compute", "doctor", "local", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ready"], true);
    assert!(body["executable"].as_str().unwrap().contains("orx"));
    assert!(!sandbox
        .run(&["compute", "doctor", "colab", "--host", "lab"])
        .status
        .success());
}

#[test]
fn imports_metrics_only_into_the_named_run_and_requires_explicit_replacement() {
    let sandbox = Sandbox::new();
    assert!(!sandbox
        .run(&["exp", "metrics", "exp", "--run", "missing"])
        .status
        .success());
    let conn = rusqlite::Connection::open(sandbox.0.join("data/orx.db")).unwrap();
    conn.execute("INSERT INTO runs (id, experiment_id, project_id, status, backend_json, created_at, updated_at) VALUES ('run', 'exp', 'project', 'done', '{}', 0, 0)", []).unwrap();
    let path = sandbox.0.join("metrics.json");
    let payload = json!({"version":1,"dataset":"fixture-v1","split":"test","metrics":{"accuracy":0.9},"parameters":{"batch":32},"seed":0});
    std::fs::write(&path, payload.to_string()).unwrap();
    let args = [
        "exp",
        "metrics",
        "exp",
        "--run",
        "run",
        "--file",
        path.to_str().unwrap(),
    ];
    let output = sandbox.run(&args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        payload
    );
    assert!(!sandbox.run(&args).status.success());
    let mut replacement = args.to_vec();
    replacement.push("--replace");
    assert!(sandbox.run(&replacement).status.success());
    assert!(!sandbox
        .run(&["exp", "metrics", "wrong", "--run", "run"])
        .status
        .success());
    std::fs::write(&path, "{\"version\":2}").unwrap();
    assert!(!sandbox.run(&replacement).status.success());
    let saved = sandbox.run(&["exp", "metrics", "exp", "--run", "run"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&saved.stdout).unwrap(),
        payload
    );
}

#[test]
fn deep_research_and_role_references_are_discoverable_from_cli() {
    let sandbox = Sandbox::new();
    for name in [
        "deep-research",
        "deep-research/references/researcher.md",
        "deep-research/references/report-writer.md",
    ] {
        let output = sandbox.run(&["skill", name]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
    }
}

#[test]
fn recorded_runs_rank_against_the_root_baseline_and_warn_when_worse() {
    let sandbox = Sandbox::new();
    // Opens the store, so the schema exists before the fixture rows go in.
    assert!(!sandbox
        .run(&["results", "view", "project"])
        .status
        .success());
    let conn = rusqlite::Connection::open(sandbox.0.join("data/orx.db")).unwrap();
    conn.execute_batch(
        "INSERT INTO local_projects (id, name, slug, github_owner, github_repo, repo_path, created_at, updated_at)
             VALUES ('project', 'Project', 'project', '', '', '/tmp/project', 0, 0);
         INSERT INTO local_experiments (id, project_id, parent_experiment_id, slug, branch_name, title, run_command, created_at, updated_at)
             VALUES ('root', 'project', NULL, 'baseline', 'main', 'Baseline CNN', 'python train.py', 0, 0),
                    ('wide', 'project', 'root', 'wide', 'orx/wide', 'Wider layers', 'python train.py', 1, 1);
         INSERT INTO runs (id, experiment_id, project_id, status, backend_json, created_at, updated_at)
             VALUES ('base-run', 'root', 'project', 'done', '{}', 1, 1),
                    ('wide-run', 'wide', 'project', 'done', '{}', 2, 2);",
    )
    .unwrap();
    let record = |exp: &str, run: &str, accuracy: f64| {
        let path = sandbox.0.join(format!("{run}.json"));
        let payload = json!({"version":1,"dataset":"cifar10","split":"test","metrics":{"accuracy":accuracy,"loss":1.0 - accuracy},"seed":0});
        std::fs::write(&path, payload.to_string()).unwrap();
        sandbox.run(&[
            "exp",
            "metrics",
            exp,
            "--run",
            run,
            "--file",
            path.to_str().unwrap(),
        ])
    };
    let base = record("root", "base-run", 0.8);
    assert!(
        base.status.success(),
        "{}",
        String::from_utf8_lossy(&base.stderr)
    );
    let wide = record("wide", "wide-run", 0.75);
    assert!(wide.status.success());
    let comparison = String::from_utf8_lossy(&wide.stderr);
    assert!(
        comparison.starts_with("Warning:") && comparison.contains("worse"),
        "{comparison}"
    );

    let view = sandbox.run(&["results", "view", "project", "--json"]);
    assert!(
        view.status.success(),
        "{}",
        String::from_utf8_lossy(&view.stderr)
    );
    let report: Value = serde_json::from_slice(&view.stdout).unwrap();
    assert_eq!(report["baseline"]["runId"], "base-run");
    assert_eq!(report["primaryMetric"], "accuracy");
    assert_eq!(report["goals"]["loss"], "min");
    assert_eq!(report["rows"][0]["runId"], "base-run");
    assert_eq!(report["rows"][1]["verdict"], "worse");
    assert_eq!(report["warnings"][0]["kind"], "worseThanBaseline");

    let set = sandbox.run(&[
        "results",
        "set",
        "project",
        "--baseline-run",
        "wide-run",
        "--primary",
        "loss",
    ]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let text = String::from_utf8_lossy(&set.stdout);
    assert!(text.contains("Ranked by: loss (lower is better)"), "{text}");
    assert!(!sandbox
        .run(&["results", "set", "project", "--baseline-run", "missing"])
        .status
        .success());
    assert!(!sandbox
        .run(&[
            "results",
            "set",
            "project",
            "--baseline-run",
            "wide-run",
            "--baseline-root"
        ])
        .status
        .success());
}
