import { Check, Code2, Copy, Download, Shapes } from "lucide-react";
import { useEffect, useId, useMemo, useState } from "react";
import { m } from "../paraglide/messages.js";
import { DIAGRAM_FONT, sanitizeSvgMarkup, svgDataUrl, type DiagramKind } from "../diagramSvg";
import { useResolvedTheme } from "../theme";
import { IconButton, IconButtonLink } from "./ui";

// Streaming grows a fence a few tokens at a time; draw once it settles.
const SETTLE_MS = 350;
// Transcript rows remount as they scroll; keep recent renders.
const CACHE_LIMIT = 48;
const cache = new Map<string, string>();

function remember(key: string, svg: string) {
  cache.delete(key);
  cache.set(key, svg);
  if (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!);
}

type MermaidApi = typeof import("mermaid").default;
let mermaidLoad: Promise<MermaidApi> | null = null;
let mermaidTheme: string | null = null;
// mermaid.render shares one hidden sandbox element, so renders run one at a time.
let renderQueue: Promise<unknown> = Promise.resolve();

function loadMermaid(): Promise<MermaidApi> {
  // Loaded on first use: it is large and most chats never draw a diagram.
  mermaidLoad ??= import("mermaid").then(mod => mod.default);
  return mermaidLoad;
}

async function renderMermaid(code: string, theme: "light" | "dark", id: string): Promise<string> {
  const mermaid = await loadMermaid();
  const run = renderQueue.then(async () => {
    if (mermaidTheme !== theme) {
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        // Diagrams cannot override these through %%{init}%% directives.
        secure: ["secure", "securityLevel", "startOnLoad", "maxTextSize", "themeCSS", "fontFamily", "htmlLabels", "dompurifyConfig"],
        theme: theme === "dark" ? "dark" : "default",
        // Measure labels with the faces the <img> will paint them in.
        fontFamily: DIAGRAM_FONT,
        maxTextSize: 50_000,
        suppressErrorRendering: true,
      });
      mermaidTheme = theme;
    }
    const { svg } = await mermaid.render(id, code);
    return svg;
  });
  renderQueue = run.catch(() => undefined);
  return run;
}

type Rendered = { status: "pending" } | { status: "ready"; svg: string } | { status: "error" };

function useDiagram(kind: DiagramKind, code: string): Rendered {
  const theme = useResolvedTheme();
  const reactId = useId();
  const key = `${kind}:${theme}:${code}`;
  const [state, setState] = useState<Rendered>(() => {
    const hit = cache.get(key);
    return hit ? { status: "ready", svg: hit } : { status: "pending" };
  });
  useEffect(() => {
    const hit = cache.get(key);
    if (hit) {
      setState({ status: "ready", svg: hit });
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const raw = kind === "svg"
          ? code
          : await renderMermaid(code, theme, `orx-diagram-${reactId.replace(/[^\w-]/g, "")}-${Date.now().toString(36)}`);
        const svg = sanitizeSvgMarkup(raw);
        if (cancelled) return;
        if (!svg) throw new Error("empty diagram");
        remember(key, svg);
        setState({ status: "ready", svg });
      } catch {
        if (!cancelled) setState({ status: "error" });
      }
    }, SETTLE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [key, kind, code, theme, reactId]);
  return state;
}

/** A ```svg or ```mermaid fence drawn as a picture, with the source one
 * click away. Falls back to the source when the markup does not render
 * (including mid-stream, while the fence is still incomplete). */
export function DiagramBlock({ code, kind, source }: { code: string; kind: DiagramKind; source: React.ReactNode }) {
  const rendered = useDiagram(kind, code);
  const [showSource, setShowSource] = useState(false);
  const [copied, setCopied] = useState(false);
  const ready = rendered.status === "ready" ? rendered.svg : null;
  const imageUrl = useMemo(() => ready ? svgDataUrl(ready) : null, [ready]);
  const copy = () => {
    navigator.clipboard?.writeText(code).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  const label = m.md_diagram_label();
  return (
    <div className="md-diagram my-2.5 mx-0 overflow-hidden rounded-md border border-border-muted bg-surface [&_pre]:m-0 [&_pre]:rounded-none [&_pre]:border-0">
      <div className="flex items-center gap-1 border-b border-border-muted px-2 py-1 text-xs text-subtext">
        <Shapes size={13} className="flex-none" />
        <span className="me-auto font-medium">{kind === "mermaid" ? `${label} · Mermaid` : `${label} · SVG`}</span>
        {imageUrl && <IconButton size="small" title={showSource ? m.md_diagram_show_diagram() : m.md_diagram_show_source()}
          aria-label={showSource ? m.md_diagram_show_diagram() : m.md_diagram_show_source()} aria-pressed={showSource}
          onClick={() => setShowSource(value => !value)}>
          {showSource ? <Shapes size={14} /> : <Code2 size={14} />}
        </IconButton>}
        <IconButton size="small" title={m.md_copy()} aria-label={m.md_copy_code()} onClick={copy}>
          {copied ? <Check size={14} /> : <Copy size={14} />}
        </IconButton>
        {imageUrl && <IconButtonLink size="small" href={imageUrl} download="diagram.svg" title={m.md_diagram_download()} aria-label={m.md_diagram_download()}>
          <Download size={14} />
        </IconButtonLink>}
      </div>
      {imageUrl && !showSource
        ? <div className={`flex justify-center p-3 ${kind === "svg" ? "bg-white" : "bg-background"}`}>
            <img src={imageUrl} alt={label} draggable={false} className="block h-auto max-h-[min(560px,70vh)] w-auto max-w-full" />
          </div>
        : <>
            {rendered.status !== "ready" && <div className="px-3 pt-2 text-xs text-subtext" role="status">
              {rendered.status === "error" ? m.md_diagram_error() : m.md_diagram_rendering()}
            </div>}
            {source}
          </>}
    </div>
  );
}
