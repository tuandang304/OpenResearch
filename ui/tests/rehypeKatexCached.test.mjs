import assert from "node:assert/strict";
import { realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import test from "node:test";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import remarkRehype from "remark-rehype";
import rehypeKatex from "rehype-katex";
import { KATEX_ROOT_TAG, rehypeKatexCached } from "../src/rehypeKatexCached.ts";

// rehype-katex's own HTML parser, so the expansion below is exactly its step.
const katexDir = realpathSync(fileURLToPath(new URL("../node_modules/rehype-katex/package.json", import.meta.url)));
const { fromHtmlIsomorphic } = await import(pathToFileURL(createRequire(katexDir).resolve("hast-util-from-html-isomorphic")).href);

const pipeline = (katexPlugin) => unified().use(remarkParse).use(remarkGfm)
  .use(remarkMath, { singleDollarTextMath: true }).use(remarkRehype).use(katexPlugin);

/** Replace each `katex-root` with the hast rehype-katex builds from the same markup. */
function expand(node) {
  if (!node.children) return node;
  node.children = node.children.flatMap((child) => {
    if (child.type === "element" && child.tagName === KATEX_ROOT_TAG) {
      const [rootClass] = child.properties.className;
      return fromHtmlIsomorphic(`<span class="${rootClass}">${child.properties.html}</span>`, { fragment: true }).children;
    }
    return [expand(child)];
  });
  return node;
}

const strip = (tree) => JSON.parse(JSON.stringify(tree, (key, value) => key === "position" ? undefined : value));

const samples = [
  "Inline $x^2 + y_i$ and $\\frac{a}{b}$ in a sentence.",
  "$$\n\\int_0^1 x\\,dx = \\tfrac12\n$$",
  "```math\nE = mc^2\n```",
  "Repeated $x^2$ and again $x^2$, display\n\n$$\nx^2\n$$",
  "| a | $\\alpha$ |\n|---|---|\n| 1 | $\\beta$ |",
  "- item with $\\sqrt{2}$\n  > quoted $\\pi$",
  "Broken $\\frac{1}{$ math and an unknown $\\notacommand$ macro.",
  "No math at all, just **bold** and `code`.",
];

test("rehypeKatexCached renders the same hast as rehype-katex", () => {
  const original = pipeline(rehypeKatex);
  const cached = pipeline(rehypeKatexCached);
  for (const markdown of samples) {
    const expected = strip(original.runSync(original.parse(markdown)));
    // Twice: the second run is served from the cache.
    for (let pass = 0; pass < 2; pass++) {
      const actual = strip(expand(cached.runSync(cached.parse(markdown))));
      assert.deepEqual(actual, expected, `${markdown} (pass ${pass})`);
    }
  }
});

test("rehypeKatexCached reuses the rendered node for a repeated formula", () => {
  const cached = pipeline(rehypeKatexCached);
  const roots = [];
  const collect = (node) => {
    if (node.tagName === KATEX_ROOT_TAG) roots.push(node);
    node.children?.forEach(collect);
  };
  collect(cached.runSync(cached.parse("$a+b$ then $a+b$")));
  collect(cached.runSync(cached.parse("later: $a+b$")));
  assert.equal(roots.length, 3);
  assert.ok(roots.every((root) => root === roots[0]));
  assert.deepEqual(roots[0].properties.className, ["katex"]);
});
