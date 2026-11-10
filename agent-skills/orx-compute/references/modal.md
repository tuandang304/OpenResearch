# Modal (`--backend modal`)

Use this backend only when the user explicitly requests Modal or it is the
configured default (or the playbook's compute routing lists it). Modal runs an
ephemeral Sandbox in the user's account and bills that account per second.

Authentication uses `MODAL_TOKEN_ID` and `MODAL_TOKEN_SECRET`, or credentials
created by `modal token new`. `orx` provisions its managed Modal environment on
the first launch.

```sh
orx exp run <expId> --backend modal --flavor a10g
orx exp run <expId> --backend modal --flavor a100-80gb --timeout 8h
orx exp run <expId> --backend modal --flavor h100:2
orx exp run <expId> --backend modal --flavor cpu --image python:3.12
```

- `--flavor` is required unless the configured default includes one. GPU values
  include `t4`, `l4`, `a10g`, `a100`, `a100-80gb`, `l40s`, `h100`, and `h200`;
  append `:N` for multiple GPUs. CPU values are `cpu` and `cpu-large`.
- Timeout defaults to 4 hours and covers the whole Sandbox.
- `--image` overrides the default CUDA PyTorch or CPU Python image. `orx`
  adds `uv` to every image's PATH; an image's own `uv` on its PATH takes precedence.
- `orx` copies the committed snapshot into the Sandbox; Modal never needs
  repository access.
- `$ORX_CHECKPOINT_DIR` is where a run saves checkpoints. With checkpoint
  storage on (`orx compute checkpoints show`), it syncs to the user's rclone
  remote, such as Google Drive, every few minutes and when the run ends, and
  the next run of the same experiment starts with it restored. A Sandbox cut
  off by its timeout keeps only the last periodic sync. Save checkpoints there,
  write each to a temporary name and rename it when complete, and resume from
  the latest one found at start. With it off, the run log says so and the
  folder dies with the Sandbox; tell the user that `orx compute checkpoints
  set <remote:folder>` keeps it. Only the user sets it up, since it needs
  `rclone config` on their machine.

See [run monitoring](../SKILL.md#run-engine-setting).
