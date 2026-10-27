// Agent-drawn diagrams in chat: ```svg and ```mermaid fences render as
// pictures. The markup comes from model output (which may echo untrusted
// papers or web pages), so it is reduced to an allowlist of drawing elements
// and attributes before it reaches the page: no scripts, no event handlers,
// no external references, no embedded HTML beyond plain text containers.

/** System faces only: an SVG drawn through <img> cannot load the page's fonts. */
export const DIAGRAM_FONT = '-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, Roboto, "Helvetica Neue", sans-serif';

export type DiagramKind = "svg" | "mermaid";

/** The fence languages that render as diagrams. */
export function diagramKind(lang: string | null | undefined): DiagramKind | null {
  const value = lang?.trim().toLowerCase();
  if (value === "svg") return "svg";
  if (value === "mermaid" || value === "mmd") return "mermaid";
  return null;
}

const SVG_ELEMENTS = new Set([
  "svg", "g", "defs", "symbol", "use", "title", "desc", "style",
  "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
  "text", "tspan", "textpath",
  "lineargradient", "radialgradient", "stop", "pattern", "marker", "clippath", "mask",
  "filter", "fegaussianblur", "feoffset", "feblend", "feflood", "fecomposite",
  "femerge", "femergenode", "fedropshadow", "fecolormatrix",
  "foreignobject",
]);

// Mermaid writes node labels as HTML inside <foreignObject>; only inert text
// containers survive there.
const LABEL_ELEMENTS = new Set(["div", "span", "p", "br", "b", "i", "strong", "em", "code", "sub", "sup"]);

const DROPPED_WITH_CONTENT = new Set(["script", "iframe", "object", "embed", "image", "img", "video", "audio", "animate", "animatemotion", "animatetransform", "set", "a", "link", "meta", "base", "form", "input", "button", "textarea", "select"]);

const ATTRIBUTES = new Set([
  "id", "class", "style", "role", "aria-label", "aria-labelledby", "aria-roledescription",
  "xmlns", "xmlns:xlink", "version", "viewbox", "width", "height", "preserveaspectratio",
  "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "dx", "dy", "d", "points",
  "transform", "pathlength", "fill", "fill-opacity", "fill-rule", "clip-rule", "stroke",
  "stroke-width", "stroke-opacity", "stroke-dasharray", "stroke-dashoffset", "stroke-linecap",
  "stroke-linejoin", "stroke-miterlimit", "opacity", "color", "display", "visibility",
  "font-family", "font-size", "font-weight", "font-style", "text-anchor", "dominant-baseline",
  "alignment-baseline", "baseline-shift", "letter-spacing", "word-spacing", "text-decoration",
  "writing-mode", "rotate", "textlength", "lengthadjust", "startoffset",
  "offset", "stop-color", "stop-opacity", "gradientunits", "gradienttransform", "spreadmethod",
  "fx", "fy", "fr", "patternunits", "patterncontentunits", "patterntransform",
  "markerwidth", "markerheight", "markerunits", "refx", "refy", "orient",
  "clippathunits", "maskunits", "maskcontentunits", "clip-path", "mask", "filter",
  "marker-start", "marker-mid", "marker-end", "filterunits", "primitiveunits",
  "stddeviation", "in", "in2", "result", "mode", "operator", "k1", "k2", "k3", "k4",
  "flood-color", "flood-opacity", "values", "type", "href", "xlink:href",
  "data-id", "data-node", "data-et", "data-edge", "data-look",
]);

/** `href`s may only point inside the document (`#marker`). */
export function isSafeReference(value: string): boolean {
  return value.trim().startsWith("#");
}

/** Paint, clip and marker values may reference fragments, never URLs. */
export function isSafeAttributeValue(value: string): boolean {
  const lower = value.toLowerCase().replace(/[\s\\]/g, "");
  if (/javascript:|vbscript:|data:|expression\(/.test(lower)) return false;
  for (const match of lower.matchAll(/url\(([^)]*)\)/g)) {
    if (!match[1].replace(/["']/g, "").startsWith("#")) return false;
  }
  return true;
}

/** CSS from a <style> element or style attribute, minus anything that loads
 * resources or reaches outside the drawing. */
export function sanitizeCss(css: string): string {
  return css
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/@import[^;]*;?/gi, "")
    .replace(/url\(\s*(['"]?)(?!#)[^)]*\1\s*\)/gi, "none")
    .replace(/expression\s*\(/gi, "(")
    .replace(/(?:java|vb)script\s*:/gi, "")
    .replace(/behavior\s*:/gi, "")
    .replace(/-moz-binding\s*:/gi, "");
}

/** The slice of the DOM used by the sanitizer, so tests can drive it without
 * a browser. */
export interface SanitizableElement {
  readonly localName: string;
  readonly attributes: ArrayLike<{ name: string; value: string }>;
  readonly children: ArrayLike<SanitizableElement>;
  textContent: string | null;
  removeAttribute(name: string): void;
  setAttribute(name: string, value: string): void;
  remove(): void;
}

/** Strips `root` in place to the allowlist. Returns false when the root
 * itself is not an `<svg>`. */
export function sanitizeSvgTree(root: SanitizableElement): boolean {
  if (root.localName.toLowerCase() !== "svg") return false;
  sanitizeElement(root, false);
  return true;
}

function sanitizeElement(element: SanitizableElement, insideLabel: boolean) {
  const name = element.localName.toLowerCase();
  const allowed = insideLabel ? LABEL_ELEMENTS.has(name) : SVG_ELEMENTS.has(name);
  if (!allowed || DROPPED_WITH_CONTENT.has(name)) {
    element.remove();
    return;
  }
  for (const { name: attr, value } of Array.from(element.attributes)) {
    const key = attr.toLowerCase();
    const keep = insideLabel
      ? key === "class" || key === "style"
      : ATTRIBUTES.has(key) && (key === "href" || key === "xlink:href" ? isSafeReference(value) : isSafeAttributeValue(value));
    if (!keep || key.startsWith("on")) element.removeAttribute(attr);
    else if (key === "style") element.setAttribute(attr, sanitizeCss(value));
  }
  if (name === "style") {
    element.textContent = sanitizeCss(element.textContent ?? "");
    return;
  }
  const childInsideLabel = insideLabel || name === "foreignobject";
  for (const child of Array.from(element.children)) sanitizeElement(child, childInsideLabel);
}

/** Parses agent-written SVG leniently (HTML parsing never runs scripts) and
 * returns sanitized standalone markup, or null when there is no <svg>. */
export function sanitizeSvgMarkup(markup: string): string | null {
  const doc = new DOMParser().parseFromString(`<!doctype html><body>${markup}`, "text/html");
  const svg = doc.body.querySelector("svg");
  if (!svg || !sanitizeSvgTree(svg)) return null;
  if (!svg.getAttribute("xmlns")) svg.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  // As an image the drawing falls back to a serif face; match the app instead.
  if (!svg.getAttribute("font-family")) svg.setAttribute("font-family", DIAGRAM_FONT);
  return new XMLSerializer().serializeToString(svg);
}

/** A standalone SVG as an image URL: rendered through <img>, its styles and
 * any surviving markup stay isolated from the page. */
export function svgDataUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
