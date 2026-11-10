// Colab, Google Cloud and Kaggle settings API, owned by the tuandang304 fork.
// Re-exported from api.ts and spread into each workspace's API; see docs/fork-maintenance.md.

export interface ColabAccelerator {
  id: string;
  kind: "cpu" | "gpu" | "tpu";
  cliValue: string;
  label: string;
  memoryGb: number;
  highMem: boolean;
}

/** Local probes of the Colab CLI; never starts its sign-in. */
export interface ColabSettings {
  supportedOs: boolean;
  cliPath: string | null;
  cliVersion: string | null;
  signedIn: boolean;
  accountCount: number;
  /** The default account's email. */
  active: string | null;
  pausedCount: number;
  /** The CLI has its own sign-in in ~/.config/colab-cli that can be imported. */
  importable: boolean;
  ready: boolean;
  error: string | null;
  defaultFlavor: string;
  accelerators: ColabAccelerator[];
  /** The default Google account; `null` until one is saved. */
  account: ColabAccount | null;
  /** Runtimes assigned to the default account right now; `null` when unknown. */
  runtimes: ColabRuntime[] | null;
  /** Every saved Google account; tokens never leave the server. */
  accounts: ColabSavedAccount[];
}

export interface ColabSavedAccount {
  /** The Google account's email. */
  id: string;
  active: boolean;
  /** Never picked by `@auto` or for runs without an account. */
  paused: boolean;
  account: ColabAccount | null;
  runtimes: ColabRuntime[] | null;
}

export interface ColabRuntime {
  endpoint: string;
  accelerator: string | null;
  highMem: boolean;
  name: string | null;
  /** The OpenResearch run that owns it, when it is one of ours. */
  runId: string | null;
  runStatus: string | null;
  /** Its run ended; the background sweep releases it. */
  orphaned: boolean;
}

export interface ColabRate {
  id: string;
  cuPerHour: number;
  /** Measured on this account; otherwise a third-party estimate. */
  measured: boolean;
}

/** Best-effort: any field Colab did not answer is `null`, with `error` saying why. */
export interface ColabAccount {
  email: string | null;
  tier: "free" | "pro" | "pro_plus" | null;
  balance: number | null;
  rateHourly: number | null;
  activeRuntimes: number | null;
  /** Accelerator ids the plan can assign; `null` when Colab did not say. */
  eligible: string[] | null;
  rates: ColabRate[];
  error: string | null;
}

export interface GcpInstance {
  name: string;
  zone: string;
  status: string;
  machineType: string;
  externalIp: string | null;
  runId: string | null;
  gpu: string | null;
  gpuCount: number;
}

export interface GcpSettings {
  settings: {
    project: string | null;
    zone: string | null;
    spot: boolean;
    imageFamily: string | null;
    imageProject: string | null;
    diskGb: number | null;
    creditUsd: number | null;
    creditAsOf: number | null;
    billingExportTable: string | null;
  };
  preflight: {
    cliPath: string | null;
    account: string | null;
    project: string | null;
    zone: string;
    computeReady: boolean;
    error: string | null;
  };
  ready: boolean;
  /** VMs orx created that still exist; `null` when they could not be listed. */
  instances: GcpInstance[] | null;
  billing: { id: string; name: string | null; open: boolean | null; enabled: boolean } | null;
  /** What is left to spend; `null` until the user enters the credit. */
  balance: {
    availableUsd: number;
    creditUsd: number;
    asOf: number;
    spentUsd: number;
    source: "export" | "estimate";
    error: string | null;
  } | null;
  gpus: { id: string; label: string; vramGb: number; usdPerHour: number }[];
}

export type GcpSettingsUpdate = {
  project?: string;
  zone?: string;
  spot?: boolean;
  creditUsd?: number;
  clearCredit?: boolean;
  billingExportTable?: string;
};

export interface KaggleAccelerator {
  id: string;
  kind: "cpu" | "gpu" | "tpu";
  machineShape: string;
  label: string;
  count: number;
  memoryGb: number;
}

export interface KaggleQuotaLine {
  usedHours: number;
  remainingHours: number;
  totalHours: number;
}

/** One saved Kaggle account; credentials never leave the server. */
export interface KaggleAccount {
  username: string;
  method: "token" | "key" | "oauth";
  /** Used when a launch names no account. */
  active: boolean;
  /** `null` when Kaggle did not say (or the CLI is too old for `kaggle quota`). */
  quota: { gpu: KaggleQuotaLine | null; tpu: KaggleQuotaLine | null; refreshAt: string | null } | null;
  error: string | null;
}

/** Local probes of the Kaggle CLI plus every saved account's weekly quota. */
export interface KaggleSettings {
  supportedOs: boolean;
  cliPath: string | null;
  cliVersion: string | null;
  cliOutdated: boolean;
  installCommands: string[];
  active: string | null;
  accountCount: number;
  /** `~/.kaggle` holds a sign-in that can be imported as an account. */
  importable: boolean;
  ready: boolean;
  error: string | null;
  defaultFlavor: string;
  accelerators: KaggleAccelerator[];
  accounts: KaggleAccount[];
}

export interface NewKaggleAccount {
  username?: string;
  token?: string;
  key?: string;
  kaggleJson?: string;
}

export interface KaggleLiveKernel {
  account: string;
  kernel: string;
  title: string;
  status: "queued" | "running";
  lastRunTime: string | null;
  url: string;
}

/** Where Google Cloud and Modal runs save `$ORX_CHECKPOINT_DIR`. */
export type CheckpointSettings =
  | { enabled: false }
  | {
      enabled: true;
      /** An rclone remote and folder, such as `gdrive:openresearch`. */
      remote: string;
      intervalMinutes: number;
      rcloneConfig: string | null;
      /** Why runs could not use the remote now, such as a remote missing from the config. */
      error: string | null;
    };

type Get = <T>(url: string, signal?: AbortSignal) => Promise<T>;
type Post = <T>(url: string, body?: unknown) => Promise<T>;

export function createForkComputeApi(get: Get, post: Post) {
  const getGcpSettings = (signal?: AbortSignal) => get<GcpSettings>("/api/settings/gcp", signal);

  const saveGcpSettings = (update: GcpSettingsUpdate) =>
    post<GcpSettings>("/api/settings/gcp", update);

  const stopGcpInstance = (zone: string, name: string) =>
    post<{ ok: true }>("/api/settings/gcp/instances/stop", { zone, name });

  const getCheckpointSettings = (signal?: AbortSignal) =>
    get<CheckpointSettings>("/api/settings/checkpoints", signal);

  /** An empty `remote` turns checkpoint storage off. */
  const saveCheckpointSettings = (remote: string, intervalMinutes?: number) =>
    post<CheckpointSettings>("/api/settings/checkpoints", { remote, intervalMinutes });

  const getColabSettings = (signal?: AbortSignal, fresh = false) =>
    get<ColabSettings>(fresh ? "/api/settings/colab?fresh=true" : "/api/settings/colab", signal);

  const stopColabRuntime = (endpoint: string, account?: string) =>
    post<{ ok: true }>("/api/settings/colab/runtimes/stop", { endpoint, account });

  /** Starts the Colab CLI's sign-in for a new account; `url` is Google's page. */
  const startColabLogin = () =>
    post<{ id: string; url: string }>("/api/settings/colab/accounts/login", {});

  const finishColabLogin = (id: string, code: string) =>
    post<ColabSettings>("/api/settings/colab/accounts/login/finish", { id, code });

  const cancelColabLogin = (id: string) =>
    post<{ ok: true }>("/api/settings/colab/accounts/login/cancel", { id });

  const importColabAccount = () =>
    post<ColabSettings>("/api/settings/colab/accounts/import", {});

  const setColabActiveAccount = (account: string) =>
    post<ColabSettings>("/api/settings/colab/accounts/active", { account });

  const pauseColabAccount = (account: string, paused: boolean) =>
    post<ColabSettings>("/api/settings/colab/accounts/pause", { account, paused });

  const removeColabAccount = (account: string) =>
    post<ColabSettings>("/api/settings/colab/accounts/remove", { account });

  const getKaggleLiveKernels = (signal?: AbortSignal, fresh = false) =>
    get<{ kernels: KaggleLiveKernel[] }>(fresh ? "/api/settings/kaggle/kernels?fresh=true" : "/api/settings/kaggle/kernels", signal)
      .then((r) => r.kernels);

  const getKaggleSettings = (signal?: AbortSignal, fresh = false) =>
    get<KaggleSettings>(fresh ? "/api/settings/kaggle?fresh=true" : "/api/settings/kaggle", signal);

  const addKaggleAccount = (account: NewKaggleAccount) =>
    post<KaggleSettings>("/api/settings/kaggle/accounts", account);

  const importKaggleAccount = () =>
    post<KaggleSettings>("/api/settings/kaggle/accounts/import", {});

  const setKaggleActiveAccount = (username: string) =>
    post<KaggleSettings>("/api/settings/kaggle/accounts/active", { username });

  const removeKaggleAccount = (username: string) =>
    post<KaggleSettings>("/api/settings/kaggle/accounts/remove", { username });

  return {
    getGcpSettings,
    saveGcpSettings,
    stopGcpInstance,
    getCheckpointSettings,
    saveCheckpointSettings,
    getColabSettings,
    stopColabRuntime,
    startColabLogin,
    finishColabLogin,
    cancelColabLogin,
    importColabAccount,
    setColabActiveAccount,
    pauseColabAccount,
    removeColabAccount,
    getKaggleLiveKernels,
    getKaggleSettings,
    addKaggleAccount,
    importKaggleAccount,
    setKaggleActiveAccount,
    removeKaggleAccount,
  };
}
