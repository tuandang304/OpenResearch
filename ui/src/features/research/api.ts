import type { RunMetrics } from "./comparison";

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

type Get = <T>(url: string, signal?: AbortSignal) => Promise<T>;
type Post = <T>(url: string, body?: unknown) => Promise<T>;

/** The fork's research endpoints, spread into each workspace's API. */
export function createResearchApi(get: Get, post: Post) {
  return {
    diagnoseCompute: (backend: string) =>
      post<DiagnosticReport>(`/api/settings/compute/doctor/${encodeURIComponent(backend)}`),
    getProjectMetrics: (projectId: string, signal?: AbortSignal) =>
      get<Record<string, RunMetrics>>(`/api/projects/${encodeURIComponent(projectId)}/metrics`, signal),
    importRunMetrics: (runId: string, metrics: unknown, replace: boolean) =>
      post<RunMetrics>(`/api/runs/${encodeURIComponent(runId)}/metrics`, { metrics, replace }),
    getForkLitSources: (signal?: AbortSignal) =>
      get<ForkLitSourcesSettings>("/api/settings/fork-lit-sources", signal),
    setForkLitSources: (body: ForkLitSourcesSettings) =>
      post<ForkLitSourcesSettings>("/api/settings/fork-lit-sources", body),
  };
}
