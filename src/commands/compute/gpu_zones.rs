//! `orx compute gpu-zones`, owned by the tuandang304 fork: which Google Cloud
//! zones offer which GPUs, from `gcloud compute accelerator-types list`.

use super::*;
use crate::jobs::gcp;

#[derive(Debug, Args)]
pub struct GpuZonesArgs {
    /// A full accelerator type (`nvidia-tesla-t4`, `nvidia-l4`, `nvidia-a100-80gb`,
    /// `nvidia-h100-80gb`) matches exactly; other text matches by substring
    /// (`a100` lists every A100). Omit to list every GPU.
    #[arg(long)]
    pub gpu: Option<String>,
    /// Google Cloud project; the saved setting, then gcloud's default.
    #[arg(long)]
    pub project: Option<String>,
}

pub(super) async fn gpu_zones(args: GpuZonesArgs, json: bool) -> Result<()> {
    let settings = gcp::load_settings()?.unwrap_or_default();
    let (configured, zone) = gcp::resolve_location(&settings).await;
    let project = args.project.or(configured).ok_or_else(|| {
        anyhow!(
            "No Google Cloud project. Pass --project, run `orx compute configure gcp --project \
             <id>`, or `gcloud config set project <id>`."
        )
    })?;
    let filter = args.gpu.as_deref().and_then(gcp::accelerator_filter);
    let offers = gcp::list_accelerator_zones(&project, filter.as_deref()).await?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "project": project,
                "preferredZone": zone,
                "offers": offers,
            }))?
        );
        return Ok(());
    }
    if offers.is_empty() {
        println!(
            "No zone of {project} offers {}.",
            args.gpu.as_deref().unwrap_or("any GPU")
        );
        return Ok(());
    }
    // One row per GPU, its zones closest to the preferred zone first.
    let mut names: Vec<&str> = offers.iter().map(|offer| offer.name.as_str()).collect();
    names.dedup();
    let rows: Vec<Vec<String>> = names
        .iter()
        .map(|name| {
            let zones: Vec<String> = offers
                .iter()
                .filter(|offer| offer.name == *name)
                .map(|offer| offer.zone.clone())
                .collect();
            let orx_id = gcp::GPUS
                .iter()
                .find(|gpu| gpu.accelerator_type == *name)
                .map_or("-", |gpu| gpu.id);
            vec![
                name.to_string(),
                orx_id.to_string(),
                zones.len().to_string(),
                gcp::order_zones(&zone, &zones).join(" "),
            ]
        })
        .collect();
    print_table(
        &["ACCELERATOR", "FLAVOR", "ZONES", "WHERE (nearest first)"],
        &rows,
    );
    println!("\nPreferred zone: {zone}. A GPU run starts there when it has the GPU, otherwise in the nearest zone that does.");
    Ok(())
}
