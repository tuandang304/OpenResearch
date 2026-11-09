import { useQuery, queryOptions } from "@tanstack/react-query";
import type { WorkspaceClient } from "./queries/client";
import { useWorkspace } from "./workspace";

/** Artifact files are fingerprinted by the server's event loop, which sends
 * `files.updated` within ~0.5s of a change and invalidates these versions, so
 * their HEAD poll is only a fallback (e.g. trees past the fingerprint cap). */
export const artifactUrlPrefix = (projectId: string) => `/api/projects/${projectId}/files/file?`;
const ARTIFACT_FALLBACK_POLL_MS = 15_000;
const isArtifactUrl = (url: string) => /^\/api\/projects\/[^/]+\/files\/file\?/.test(url);

export const fileVersionQuery = (client: WorkspaceClient, url: string) => queryOptions({
  queryKey: client.workspaceKey("fileVersion", url),
  staleTime: 2_000,
  refetchOnMount: "always",
  refetchOnWindowFocus: "always",
  refetchInterval: isArtifactUrl(url) ? ARTIFACT_FALLBACK_POLL_MS : 2_000,
  queryFn: async ({ signal }) => {
    const response = await client.request(url, { method: "HEAD", cache: "no-store", signal });
    if (response.status === 404) return "missing";
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.headers.get("etag") ?? response.headers.get("content-length");
  },
});

export function useFileVersion(url: string, enabled = true) {
  const client = useWorkspace();
  const query = useQuery({ ...fileVersionQuery(client, url), enabled, subscribed: enabled });
  return query.data ?? null;
}
