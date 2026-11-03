import type { Harness } from "./api";

export function resolveHarnessDefaultModel(harness: Harness | undefined): string | null {
  if (!harness || harness.catalogPending || harness.models.length === 0) return null;
  if (harness.id === "opencode") return harness.models[0].id;
  if (harness.id === "claude-code") {
    if (!harness.agentReady) return null;
    return groupModels(harness.id, harness.models).current.find((model) => /\bopus\b/i.test(`${model.displayName ?? ""} ${model.id}`))?.id ?? null;
  }
  if (harness.id !== "codex") return null;
  if (!harness.agentReady) return null;

  let best = harness.models[0].id;
  let bestVersion: bigint[] | null = null;
  for (const model of harness.models) {
    const match = [model.id, model.displayName ?? ""]
      .map((name) => /gpt[\s._-]*(\d+(?:[._-]\d+)*)[\s._-]*sol\b/i.exec(name))
      .find(Boolean);
    if (!match) continue;
    const version = match[1].split(/[._-]/).map((part) => BigInt(part));
    let comparison = 0;
    if (bestVersion) {
      for (let i = 0; i < Math.max(version.length, bestVersion.length); i++) {
        if ((version[i] ?? 0n) === (bestVersion[i] ?? 0n)) continue;
        comparison = (version[i] ?? 0n) > (bestVersion[i] ?? 0n) ? 1 : -1;
        break;
      }
    }
    if (!bestVersion || comparison > 0) {
      best = model.id;
      bestVersion = version;
    }
  }
  return best;
}

export function formatCodexModel(name: string): string {
  return name.replace(/\bgpt[\s_-]*(\d+(?:\.\d+)*)(?:[\s_-]+)(astra|sol|terra|luna)\b/i, (_, version, family) => `GPT-${version} ${family[0].toUpperCase()}${family.slice(1).toLowerCase()}`);
}

export function groupModels<T extends { id: string; displayName?: string }>(harness: string, models: T[]): { current: T[]; legacy: T[] } {
  const families = harness === "claude-code" ? ["mythos", "fable", "opus", "sonnet", "haiku"] : harness === "codex" ? ["astra", "sol", "terra", "luna"] : [];
  if (!families.length) return { current: models, legacy: [] };
  const classified = models.map((model, index) => {
    const text = `${model.displayName ?? ""} ${model.id}`.toLowerCase();
    const family = families.find((name) => new RegExp(`\\b${name}\\b`).test(text));
    const version = family && (harness === "codex"
      ? /gpt[\s_-]*(\d+(?:[._-]\d+)*)/i.exec(text)?.[1]
      : new RegExp(`${family}[\\s_-]+(\\d+(?:[._-]\\d+)*)`, "i").exec(text)?.[1]);
    return { model, index, family, version: version?.split(/[._-]/).map(Number) };
  });
  const compare = (a: number[], b: number[]) => {
    for (let i = 0; i < Math.max(a.length, b.length); i++) {
      const difference = (b[i] ?? 0) - (a[i] ?? 0);
      if (difference) return difference;
    }
    return 0;
  };
  classified.sort((a, b) => {
    const aRank = a.family ? families.indexOf(a.family) : families.length;
    const bRank = b.family ? families.indexOf(b.family) : families.length;
    return aRank - bRank || (a.version && b.version ? compare(a.version, b.version) : 0) || a.index - b.index;
  });
  const seen = new Set<string>();
  const current: T[] = [];
  const legacy: T[] = [];
  for (const entry of classified) {
    if (entry.family && entry.version && seen.has(entry.family)) legacy.push(entry.model);
    else {
      current.push(entry.model);
      if (entry.family && entry.version) seen.add(entry.family);
    }
  }
  return { current, legacy };
}

// Model → harness routing. Mirrors `src/local/harness/model_route.rs`: the
// backend refuses a mismatched model, and the picker uses the same rules so it
// never offers one in the first place.

export type ModelFamily = "Claude" | "OpenAI" | "Gemini" | "Grok" | "DeepSeek" | "Cursor";

const VENDOR_PREFIX = /^(anthropic|openai|google|xai|deepseek|cursor)\//;
const CLAUDE_ALIASES = new Set(["opus", "sonnet", "haiku", "fable", "opusplan"]);
const DEFAULT_IDS = new Set(["", "default", "auto"]);

export const MODEL_FAMILY_HOME: Record<ModelFamily, string> = {
  Claude: "claude-code",
  OpenAI: "codex",
  Gemini: "antigravity",
  Grok: "cursor",
  Cursor: "cursor",
  DeepSeek: "opencode",
};

/** The vendor family of `model`, or null for defaults and ids we don't recognize. */
export function modelFamily(model: string | null | undefined): ModelFamily | null {
  const id = (model ?? "").trim().split(/\s+/).join("-").toLowerCase().replace(VENDOR_PREFIX, "");
  if (DEFAULT_IDS.has(id)) return null;
  if (id.startsWith("claude") || CLAUDE_ALIASES.has(id.split("[")[0])) return "Claude";
  if (id.startsWith("gpt") || id.startsWith("codex") || /^o\d(?:$|[-\d])/.test(id)) return "OpenAI";
  if (id.startsWith("gemini")) return "Gemini";
  if (id.startsWith("grok")) return "Grok";
  if (id.startsWith("deepseek")) return "DeepSeek";
  if (id.startsWith("composer")) return "Cursor";
  return null;
}

/** Whether `harness` can run models of `family`. Multi-provider harnesses take all. */
export function harnessServesFamily(harness: string, family: ModelFamily): boolean {
  if (harness === "claude-code") return family === "Claude";
  if (harness === "codex") return family === "OpenAI";
  if (harness === "antigravity") return family === "Gemini" || family === "Claude" || family === "OpenAI";
  return true;
}

/**
 * When `model` belongs to a vendor `harness` cannot run, the family and the
 * harness that does run it; null when the pair is fine (or the model unknown).
 */
export function modelHarnessMismatch(
  harness: string | null | undefined,
  model: string | null | undefined,
): { family: ModelFamily; home: string } | null {
  if (!harness) return null;
  const family = modelFamily(model);
  if (!family || harnessServesFamily(harness, family)) return null;
  return { family, home: MODEL_FAMILY_HOME[family] };
}
