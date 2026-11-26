# Google Drive storage

The user may link a Google Drive folder for datasets, checkpoints and other
large files. It is off unless the user turns it on, and Drive is slower than
the other storage, so use it only when the user asks to save something to
Drive (or to fetch something from it).

```sh
orx compute drive show --json          # linked folder, on/off, and what stops it working
orx compute drive ls [folder]          # list the linked folder or a subfolder
orx compute drive push <local> --to <folder>   # a file lands at <folder>/<name>, a directory at <folder>/<dirname>/
orx compute drive pull <path> --to <local-dir> # a file or a whole folder
```

- Paths are relative to the linked folder; subfolders are created as needed and
  nothing outside the linked folder can be reached.
- When storage is off or no folder is linked, these commands fail and say so.
  Tell the user: they turn it on under Google Drive storage in the compute
  settings.
  Only when the user asks you in this chat, and gives the folder link, run
  `orx compute drive set <folder-link> --on` yourself.
- It needs rclone with a Google Drive remote on this machine (`rclone config`,
  storage `drive`, scope `drive`). Only the user can sign in; if `show` reports
  a problem, pass its message on.
- These commands run on this machine. To save a run's outputs, first bring
  them here (for example its downloaded output folder), then push. Pick a clear
  layout such as `datasets/<name>` and `checkpoints/<experiment>`, and tell the
  user where files went.
- Uploads of many gigabytes can take a long time; say so before starting, and
  prefer pushing one archive over thousands of small files.
