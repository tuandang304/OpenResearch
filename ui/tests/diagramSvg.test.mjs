import assert from "node:assert/strict";
import test from "node:test";

import { diagramKind, isSafeAttributeValue, isSafeReference, sanitizeCss, sanitizeSvgTree, svgDataUrl } from "../src/diagramSvg.ts";

// Only the DOM surface the sanitizer touches.
class El {
  constructor(localName, attributes = {}, children = [], textContent = null) {
    this.localName = localName;
    this.attrs = new Map(Object.entries(attributes));
    this.kids = [];
    this.parent = null;
    this.textContent = textContent;
    for (const child of children) this.append(child);
  }
  append(child) { child.parent = this; this.kids.push(child); }
  get attributes() { return [...this.attrs].map(([name, value]) => ({ name, value })); }
  get children() { return [...this.kids]; }
  removeAttribute(name) { this.attrs.delete(name); }
  setAttribute(name, value) { this.attrs.set(name, value); }
  remove() { this.parent.kids = this.parent.kids.filter(child => child !== this); }
}

test("svg and mermaid fences render as diagrams", () => {
  assert.equal(diagramKind("svg"), "svg");
  assert.equal(diagramKind("Mermaid"), "mermaid");
  assert.equal(diagramKind("mmd"), "mermaid");
  assert.equal(diagramKind("xml"), null);
  assert.equal(diagramKind(undefined), null);
});

test("sanitizer keeps drawing markup and drops active content", () => {
  const circle = new El("circle", { cx: "5", cy: "5", r: "4", fill: "url(#grad)", onclick: "alert(1)" });
  const script = new El("script", {}, [], "alert(1)");
  const link = new El("a", { href: "javascript:alert(1)" }, [new El("text", {}, [], "click")]);
  const image = new El("image", { href: "https://example.com/x.png" });
  const use = new El("use", { href: "#marker" });
  const remoteUse = new El("use", { "xlink:href": "https://example.com/s.svg#a" });
  const label = new El("foreignObject", { width: "10", height: "10" }, [
    new El("div", { class: "label", onmouseover: "x()" }, [new El("span", {}, [], "Node"), new El("iframe", { src: "https://x" })]),
  ]);
  const style = new El("style", {}, [], "@import url(https://evil.css); .a{fill:url(https://x/y);stroke:url(#ok)}");
  const svg = new El("svg", { viewBox: "0 0 10 10", onload: "alert(1)", style: "background:url(https://x)" },
    [style, circle, script, link, image, use, remoteUse, label]);

  assert.equal(sanitizeSvgTree(svg), true);
  assert.deepEqual(svg.kids.map(child => child.localName), ["style", "circle", "use", "use", "foreignObject"]);
  assert.equal(svg.attrs.has("onload"), false);
  assert.equal(svg.attrs.has("style"), false);
  assert.equal(circle.attrs.has("onclick"), false);
  assert.equal(circle.attrs.get("fill"), "url(#grad)");
  assert.equal(use.attrs.get("href"), "#marker");
  assert.equal(remoteUse.attrs.has("xlink:href"), false);
  const div = label.kids[0];
  assert.equal(div.attrs.has("onmouseover"), false);
  assert.deepEqual(div.kids.map(child => child.localName), ["span"]);
  assert.equal(style.textContent.includes("evil"), false);
  assert.match(style.textContent, /stroke:url\(#ok\)/);
  assert.equal(style.textContent.includes("https"), false);
});

test("non-svg roots are rejected", () => {
  assert.equal(sanitizeSvgTree(new El("div")), false);
});

test("attribute and css guards", () => {
  assert.equal(isSafeReference("#a"), true);
  assert.equal(isSafeReference("https://x"), false);
  assert.equal(isSafeAttributeValue("url(#a)"), true);
  assert.equal(isSafeAttributeValue("url('https://x')"), false);
  assert.equal(isSafeAttributeValue("java\\script:alert(1)"), false);
  assert.equal(isSafeAttributeValue("red"), true);
  assert.equal(sanitizeCss("a{b:expression(alert(1))}").includes("expression("), false);
  assert.equal(sanitizeCss("/* @import url(x) */ .a{}"), " .a{}");
});

test("data URLs encode the markup", () => {
  assert.equal(svgDataUrl('<svg a="#"/>'), "data:image/svg+xml;charset=utf-8,%3Csvg%20a%3D%22%23%22%2F%3E");
});
