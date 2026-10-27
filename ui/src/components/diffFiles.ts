import { parseDiff, type FileData } from "react-diff-view";

// Diff parsing and counts, apart from GitDiff.tsx so callers that only need
// numbers don't load the diff renderer and its syntax highlighter.

export function countChanges(file: FileData) {
  let additions = 0;
  let deletions = 0;
  for (const hunk of file.hunks) {
    for (const change of hunk.changes) {
      if (change.type === "insert") additions++;
      else if (change.type === "delete") deletions++;
    }
  }
  return { additions, deletions };
}

export function parseDiffFiles(diff: string, partial: boolean): { files: FileData[]; failed: boolean } {
  if (!diff.trim()) return { files: [], failed: false };
  try {
    return { files: parseDiff(diff, { nearbySequences: "zip" }), failed: false };
  } catch {
    if (partial) {
      const starts = Array.from(diff.matchAll(/^diff --git /gm), (match) => match.index);
      const lastStart = starts[starts.length - 1];
      if (starts.length > 1 && lastStart !== undefined) {
        try {
          return {
            files: parseDiff(diff.slice(0, lastStart), { nearbySequences: "zip" }),
            failed: false,
          };
        } catch {
          return { files: [], failed: true };
        }
      }
    }
    return { files: [], failed: true };
  }
}
