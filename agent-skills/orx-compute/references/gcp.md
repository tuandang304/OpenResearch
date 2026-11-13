# Google Cloud (`--backend gcp`)

Use this backend only when the user explicitly requests Google Cloud or it is
the configured default (or the playbook's compute routing lists it). It creates
a Compute Engine VM with a GPU in the user's own Google Cloud project, runs the
experiment on it, and deletes the VM when the run ends. It requires the Google
Cloud CLI signed in (`gcloud auth login`) and a project with the Compute Engine
API on and GPU quota in the zone.

```sh
orx compute test gcp                    # gcloud, account, project, zone, API
orx compute gpu-zones --gpu nvidia-tesla-t4   # zones that offer a GPU
orx exp run <expId> --backend gcp --flavor t4 --timeout 2h
orx exp run <expId> --backend gcp --flavor a100:2 --timeout 8h
orx exp run <expId> --backend gcp --flavor n2-standard-16   # CPU only
```

- `--flavor` is a GPU id with an optional count: `t4`, `p4`, `l4`, `p100`,
  `v100`, `a100` (40 GB), `a100-80gb`, or `h100`, such as `a100:2`. `cpu` or a
  CPU machine type such as `n2-standard-16` gives a VM without a GPU. Without
  `--flavor` the run gets one T4.
- Size the GPU to the run: peak GPU memory against the GPU's memory
  (`orx compute prices --json` lists it with the price), then the cheapest
  GPU that fits. Do not pick an A100 or H100 for a run a T4 or L4 can hold.
- The project, zone, Spot VMs, image, and disk size come from the user's saved
  settings (`orx compute show gcp`). Change them only when the user asks:
  `orx compute configure gcp --project <id> --zone <zone>`.
- `orx compute prices --json` shows the Google Cloud balance when the user
  has entered their remaining credit: `balance.amount` is what is left in USD
  and each offer's `runwayHours` is how long it lasts. Treat it as an
  estimate; never launch a run that would outlast it.
- Spot VMs cost far less but Google can reclaim them mid-run. When the user
  has Spot on, a run must save checkpoints to `$ORX_CHECKPOINT_DIR` and resume
  from the latest one.
- The VM runs a Deep Learning VM image with CUDA, Python, and the NVIDIA
  driver, which finishes installing on first boot; orx waits for it before the
  run starts. Install project dependencies from the lockfile in the run
  command.
- Timeout defaults to 4 hours. Compute Engine deletes the VM on its own one
  hour after the timeout even if orx cannot, and `orx up` deletes VMs whose run
  has ended. The VM and its disk disappear with the run, so all evidence must
  reach the run log or `$ORX_CHECKPOINT_DIR`.
- `$ORX_CHECKPOINT_DIR` is where a run saves checkpoints. With checkpoint
  storage on (`orx compute checkpoints show`), it syncs to the user's rclone
  remote, such as Google Drive, every few minutes and when the run ends, even
  after a timeout, and the next run of the same experiment starts with it
  restored. Save checkpoints there, write each to a temporary name and rename
  it when complete, and resume from the latest one found at start. With it
  off, the run log says so and the folder dies with the VM; tell the user that
  `orx compute checkpoints set <remote:folder>` keeps it. Only the user sets it
  up, since it needs `rclone config` on their machine.
- A GPU run starts in the configured zone when it offers that GPU; otherwise
  OpenResearch looks up the zones that do (`gcloud compute accelerator-types list`)
  and creates the VM in the nearest one, moving on to the next nearest when a
  zone is out of stock. `orx compute gpu-zones --gpu <type or text>` shows
  the same lookup; `--gpu a100` matches every A100.
- "No zone had a free ..." means every nearby zone was out of stock: try again
  later, a different GPU, or another provider. "Quota exceeded" means the
  project needs more GPU quota, which only the user can request.
- A controller on this machine starts the run, records its status and logs,
  and deletes the VM; do not kill it. Cancelling the run deletes the VM.
