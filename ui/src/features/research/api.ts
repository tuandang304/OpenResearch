export interface DiagnosticReport {
  backend: string;
  ready: boolean;
  checkedAt: number;
  version: string;
  executable: string;
  buildChannel: string;
  checks: { id: string; status: "pass" | "fail"; message: string; action: string | null }[];
}

/** arXiv and Semantic Scholar toggles; `/api/settings/lit-sources` keeps upstream's shape. */
export type ForkLitSourcesSettings = Record<"arxiv" | "semanticscholar", boolean>;

/** A run's final measurements, recorded with `orx exp metrics`. */
export interface RunMetrics {
  version: 1;
  dataset: string;
  split: string;
  metrics: Record<string, number>;
  parameters?: Record<string, string | number | boolean | null>;
  seed: number | null;
}

export type Goal = "max" | "min";

export interface ResultsSettings {
  baselineRunId?: string;
  manualBaseline?: { name: string; measurement: RunMetrics };
  goals?: Record<string, Goal>;
  primaryMetric?: string;
  /** Fields a newer orx wrote; sent back unchanged. */
  [field: string]: unknown;
}

export type Verdict = "baseline" | "better" | "worse" | "same" | "incomparable" | "unknown";

export interface ResultRow {
  runId: string;
  experimentId: string;
  experimentTitle: string;
  status: string;
  commitSha: string | null;
  createdAt: number;
  endedAt: number | null;
  measurement: RunMetrics;
  deltas: Record<string, number>;
  verdict: Verdict;
  rank: number | null;
}

export type ResultsWarning =
  | { kind: "worseThanBaseline"; runId: string; experimentTitle: string; metric: string; value: number; baseline: number; delta: number }
  | { kind: "incomparable"; runId: string; experimentTitle: string; dataset: string; split: string }
  | { kind: "unmeasured"; count: number }
  | { kind: "baselineUnmeasured"; runId: string }
  | { kind: "noBaseline" };

export interface ResultsReport {
  projectId: string;
  settings: ResultsSettings;
  baseline: {
    source: "pinned" | "root" | "manual";
    name: string;
    runId: string | null;
    experimentId: string | null;
    measurement: RunMetrics | null;
  } | null;
  primaryMetric: string | null;
  goals: Record<string, Goal>;
  rows: ResultRow[];
  unmeasured: { runId: string; experimentId: string; experimentTitle: string; endedAt: number | null }[];
  warnings: ResultsWarning[];
}

type Get = <T>(url: string, signal?: AbortSignal) => Promise<T>;
type Post = <T>(url: string, body?: unknown) => Promise<T>;
type Put = <T>(url: string, body: unknown) => Promise<T>;

/** The fork's research endpoints, spread into each workspace's API. */
export function createResearchApi(get: Get, post: Post, put: Put) {
  return {
    diagnoseCompute: (backend: string) =>
      post<DiagnosticReport>(`/api/settings/compute/doctor/${encodeURIComponent(backend)}`),
    getProjectResults: (projectId: string, signal?: AbortSignal) =>
      get<ResultsReport>(`/api/projects/${encodeURIComponent(projectId)}/results`, signal),
    setResultsSettings: (projectId: string, settings: ResultsSettings) =>
      put<ResultsReport>(`/api/projects/${encodeURIComponent(projectId)}/results`, settings),
    getForkLitSources: (signal?: AbortSignal) =>
      get<ForkLitSourcesSettings>("/api/settings/fork-lit-sources", signal),
    setForkLitSources: (body: ForkLitSourcesSettings) =>
      post<ForkLitSourcesSettings>("/api/settings/fork-lit-sources", body),
  };
}
