import assert from "node:assert/strict";
import test from "node:test";
import { act, createElement } from "react";
import { m } from "../src/paraglide/messages.js";
import { ResultsTab } from "../src/features/research/ResultsTab.tsx";
import { find, render, serve, settle } from "./render.mjs";

const measured = (accuracy) => ({ version: 1, dataset: "cifar10", split: "test", metrics: { accuracy, loss: 1 - accuracy }, seed: 0 });
const row = (runId, experimentTitle, accuracy, verdict, rank, deltas = {}) => ({
  runId, experimentId: `exp-${runId}`, experimentTitle, status: "done", commitSha: null,
  createdAt: Date.UTC(2026, 10, rank ?? 9), endedAt: null, measurement: measured(accuracy), deltas, verdict, rank,
});
const report = (extra = {}) => ({
  projectId: "p",
  settings: {},
  baseline: { source: "root", name: "Baseline CNN", runId: "base", experimentId: "exp-base", measurement: measured(0.8) },
  primaryMetric: "accuracy",
  goals: { accuracy: "max", loss: "min" },
  rows: [
    row("wide", "Wider layers", 0.85, "better", 1, { accuracy: 0.05, loss: -0.05 }),
    row("base", "Baseline CNN", 0.8, "baseline", 2),
    row("deep", "Deeper net", 0.7, "worse", 3, { accuracy: -0.1, loss: 0.1 }),
  ],
  unmeasured: [{ runId: "lost1234", experimentId: "exp-lost", experimentTitle: "Dropout", endedAt: 1 }],
  warnings: [
    { kind: "worseThanBaseline", runId: "deep", experimentTitle: "Deeper net", metric: "accuracy", value: 0.7, baseline: 0.8, delta: -0.1 },
    { kind: "unmeasured", count: 1 },
  ],
  ...extra,
});

async function show(initial) {
  const saved = [];
  serve((path, init) => {
    if (path === "/api/projects/p/results" && (init.method ?? "GET") === "GET") return initial;
    if (path === "/api/projects/p/results" && init.method === "PUT") {
      const settings = JSON.parse(init.body);
      saved.push(settings);
      return { ...initial, settings, goals: { ...initial.goals, ...settings.goals } };
    }
  });
  const opened = [];
  const view = await render(createElement(ResultsTab, { projectId: "p", runs: [], onOpenRun: (experimentId, runId) => opened.push([experimentId, runId]) }));
  await settle();
  return { ...view, saved, opened };
}

test("ranks runs against the baseline and warns about a worse latest run", async () => {
  const view = await show(report());
  const text = document.body.textContent;
  assert.ok(text.includes("Baseline CNN"));
  assert.ok(text.includes(m.results_baseline_root()));
  const ranked = [...document.querySelectorAll("tbody tr")].map((tr) => tr.querySelector("button").textContent.replace(/[\u2066-\u2069]/g, ""));
  assert.deepEqual(ranked, ["Wider layers", "Baseline CNN", "Deeper net"]);
  assert.ok(find("li", "Deeper net").textContent.includes("accuracy"));
  assert.ok(find("li", "Dropout"), "unrecorded runs are named");

  await act(async () => find("li", "Deeper net").querySelector("button").click());
  assert.deepEqual(view.opened, [["exp-deep", "deep"]]);
});

test("changing which way a metric improves saves it with the other settings", async () => {
  const view = await show(report({ settings: { primaryMetric: "accuracy", addedLater: 1 } }));
  await act(async () => find("button", m.results_lower_better()).click());
  await settle();
  assert.deepEqual(view.saved, [{ primaryMetric: "accuracy", addedLater: 1, goals: { accuracy: "min" } }]);
});

test("a project with nothing measured says how results appear", async () => {
  await show(report({ baseline: null, primaryMetric: null, goals: {}, rows: [], unmeasured: [], warnings: [] }));
  assert.ok(document.body.textContent.includes(m.results_empty_title()));
  assert.ok(document.body.textContent.includes(m.results_empty_description()));
});

test("a failed load says so and offers a retry", async () => {
  serve(() => Response.json({ error: "Project not found" }, { status: 404 }));
  await render(createElement(ResultsTab, { projectId: "p", runs: [], onOpenRun() {} }));
  await settle();
  assert.ok(document.body.textContent.includes("Project not found"));
  assert.ok(find("button", m.app_retry()));
});
