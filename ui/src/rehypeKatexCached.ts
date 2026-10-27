// rehype-katex, made cheap enough to run on every streamed chunk.
//
// KaTeX dominates a chat message's markdown cost twice over: typesetting each
// formula, and then turning its output (dozens of styled spans per formula)
// into hast and from hast into React elements, node by node. Streaming re-runs
// the transform on the growing tail every flush, and a virtualized row that
// scrolls back into view renders from scratch. So:
//
// - Rendered formulas are cached by (display mode, TeX source).
// - A formula becomes one `katex-root` element carrying KaTeX's own root class
//   and its inner HTML; the `katex-root` component paints it as that same root
//   <span> with the markup set directly, so the DOM is what rehype-katex would
//   have produced without React reconciling every node inside it.
// - The cached node objects are shared, never cloned (nothing after this plugin
//   mutates hast), so the streaming memoizer matches unchanged formulas by
//   identity. Keep this the last plugin in a pipeline.
//
// Anything off the common path — a math node that is not plain text, or TeX
// KaTeX rejects — falls through to rehype-katex itself, unchanged.

import katex from "katex";
import rehypeKatex from "rehype-katex";

export interface HastNode {
  type: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
  value?: string;
}

type KatexOptions = NonNullable<Parameters<typeof rehypeKatex>[0]>;
type Transform = (tree: HastNode, file: unknown) => void;

/** Tag of the element standing in for one rendered formula. */
export const KATEX_ROOT_TAG = "katex-root";

const CACHE_LIMIT = 2_000;
const KATEX_ROOT = /^<span class="(katex|katex-display)">([\s\S]*)<\/span>$/;

function classes(node: HastNode): readonly unknown[] {
  const value = node.properties?.className;
  return Array.isArray(value) ? value : [];
}

function isMath(node: HastNode): boolean {
  const list = classes(node);
  return list.includes("language-math") || list.includes("math-display") || list.includes("math-inline");
}

/** Text of a math node built from text, <pre> and <code> only (what remark-math
 * emits), else null: anything richer goes to rehype-katex as is. */
function plainText(node: HastNode): string | null {
  if (node.type === "text") return node.value ?? "";
  if (node.type !== "element" || (node.tagName !== "code" && node.tagName !== "pre")) return null;
  let text = "";
  for (const child of node.children ?? []) {
    const value = plainText(child);
    if (value === null) return null;
    text += value;
  }
  return text;
}

/** Whether rehype-katex would replace `node` when it walks to it (or, for a
 * <pre>, to its math <code>), and in which display mode. */
function mathScope(node: HastNode): { display: boolean } | null {
  if (node.type !== "element") return null;
  // ` ```math ` and `$$` blocks: rehype-katex replaces the <pre> around the code.
  if (node.tagName === "pre" && !isMath(node)) {
    const mathCode = node.children?.some((child) =>
      child.type === "element" && child.tagName === "code" && classes(child).includes("language-math"));
    return mathCode ? { display: true } : null;
  }
  return isMath(node) ? { display: classes(node).includes("math-display") } : null;
}

/** KaTeX's markup for `tex` as one `katex-root` node, or null when KaTeX
 * rejects it or its output is not the usual single root span. */
function renderRoot(tex: string, display: boolean, options: KatexOptions): HastNode | null {
  let html: string;
  try {
    html = katex.renderToString(tex, { ...options, displayMode: display, throwOnError: true });
  } catch {
    return null;
  }
  const root = KATEX_ROOT.exec(html);
  if (!root) return null;
  return { type: "element", tagName: KATEX_ROOT_TAG, properties: { className: [root[1]], html: root[2] }, children: [] };
}

export function rehypeKatexCached(options: KatexOptions = {}) {
  const fallback = rehypeKatex(options) as unknown as Transform;
  const cache = new Map<string, HastNode[]>();
  return function transform(tree: HastNode, file: unknown) {
    const visit = (parent: HastNode) => {
      const children = parent.children;
      if (!children) return;
      for (let index = 0; index < children.length; index++) {
        const child = children[index];
        const scope = mathScope(child);
        if (!scope) {
          visit(child);
          continue;
        }
        const tex = plainText(child);
        const key = tex === null ? null : `${scope.display ? "D" : "I"}\u0000${tex}`;
        let rendered = key === null ? undefined : cache.get(key);
        if (rendered) {
          cache.delete(key!);
          cache.set(key!, rendered);
        } else {
          const root = tex === null ? null : renderRoot(tex, scope.display, options);
          if (root) {
            rendered = [root];
            cache.set(key!, rendered);
            if (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!);
          } else {
            // Uncached on purpose: the rare path keeps rehype-katex's exact
            // error output and its file messages.
            const scratch: HastNode = { type: "root", children: [child] };
            fallback(scratch, file);
            rendered = scratch.children ?? [];
          }
        }
        children.splice(index, 1, ...rendered);
        index += rendered.length - 1;
      }
    };
    visit(tree);
  };
}
