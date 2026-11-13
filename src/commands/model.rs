//! `orx model <name>`: resolve a model alias or id to its canonical id and the
//! harness that serves it, optionally checking it against a given harness.

use crate::error::{anyhow, Result};
use crate::local::harness::check_model_for_harness;
use crate::local::harness::model_route::harness_display_name;
use crate::local::harness::model_route::resolve_model;

pub async fn run(args: crate::ModelArgs) -> Result<()> {
    let resolved = resolve_model(&args.name).map_err(|message| anyhow!("{message}"))?;
    let fits = args
        .harness
        .as_deref()
        .map(|harness| check_model_for_harness(harness, &resolved.model_id));

    if args.json {
        let value = serde_json::json!({
            "input": args.name,
            "modelId": resolved.model_id,
            "family": resolved.family.label(),
            "harness": resolved.harness,
            "harnessName": harness_display_name(resolved.harness),
            "checkedHarness": args.harness,
            "ok": fits.as_ref().is_none_or(Result::is_ok),
            "error": fits.as_ref().and_then(|fit| fit.as_ref().err()).map(ToString::to_string),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("model_id: {}", resolved.model_id);
        println!(
            "harness:  {} ({})",
            resolved.harness,
            harness_display_name(resolved.harness)
        );
    }

    match fits {
        Some(Err(mismatch)) => Err(anyhow!("{mismatch}")),
        _ => Ok(()),
    }
}
