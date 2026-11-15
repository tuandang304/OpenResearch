import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { AlertTriangle, ArrowDown, ArrowUp, Info, Trophy } from "lucide-react";
import type { Run } from "../../api";
import { useWorkspace } from "../../workspace";
import { m } from "../../paraglide/messages.js";
import { getLocale } from "../../paraglide/runtime.js";
import { autoDir, fmtNumber, ltr } from "../../i18n";
import { OptionPicker } from "../../components/ModelPicker";
import { WorkspaceEmptyState } from "../../components/WorkspaceEmptyState";
import { Badge, Button, LoadingRow, SEGMENTED_CLASS_NAME, showAlert, type BadgeVariant } from "../../components/ui";
import type { Goal, ResultRow, ResultsReport, ResultsSettings, ResultsWarning, Verdict } from "./api";

const AUTO_BASELINE = "";

const value = (n: number) => new Intl.NumberFormat(getLocale(), { maximumSignificantDigits: 5 }).format(n);
const signed = (n: number) => new Intl.NumberFormat(getLocale(), { maximumSignificantDigits: 4, signDisplay: "exceptZero" }).format(n);
const percent = (n: number) => new Intl.NumberFormat(getLocale(), { style: "percent", maximumFractionDigits: 1, signDisplay: "exceptZero" }).format(n);
const day = (ms: number) => new Intl.DateTimeFormat(getLocale(), { month: "short", day: "numeric" }).format(ms);
const runLabel = (row: { experimentTitle: string; runId: string }) => `${row.experimentTitle} · ${row.runId.slice(0, 8)}`;

/** Positive when the change is an improvement for `goal`. */
const gain = (delta: number, goal: Goal | undefined) => (goal === "min" ? -delta : delta);

const VERDICTS: Record<Verdict, { label: () => string; badge: BadgeVariant }> = {
  baseline: { label: m.results_baseline, badge: "default" },
  better: { label: m.results_verdict_better, badge: "success" },
  worse: { label: m.results_verdict_worse, badge: "error" },
  same: { label: m.results_verdict_same, badge: "default" },
  incomparable: { label: m.results_verdict_incomparable, badge: "warning" },
  unknown: { label: m.results_verdict_unknown, badge: "default" },
};

export function ResultsTab({ projectId, runs, onOpenRun }: {
  projectId: string;
  runs: Run[];
  onOpenRun: (experimentId: string, runId: string) => void;
}) {
  const { api, workspaceKey, queryClient } = useWorkspace();
  const key = workspaceKey("projectResults", projectId);
  // Agents record measurements from the CLI, which sends no event; poll while the tab is open.
  const report = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => api.getProjectResults(projectId, signal),
    refetchInterval: 10_000,
  });
  const runsSignature = runs.map((run) => `${run.id}:${run.status}`).join(",");
  const { refetch } = report;
  useEffect(() => { void refetch(); }, [runsSignature, refetch]);
  const save = useMutation({
    mutationFn: (settings: ResultsSettings) => api.setResultsSettings(projectId, settings),
    onSuccess: (next) => queryClient.setQueryData(key, next),
    onError: (error) => showAlert(error instanceof Error ? error.message : String(error), "error"),
  });
  const [chartMetric, setChartMetric] = useState<string | null>(null);

  if (report.isPending) return <LoadingRow />;
  if (report.isError) {
    return (
      <div className="flex flex-col items-start gap-3 p-6 text-sm text-subtext" role="alert">
        <p className="m-0">{m.common_failed_to_load({ error: report.error.message })}</p>
        <Button size="small" onClick={() => void report.refetch()}>{m.app_retry()}</Button>
      </div>
    );
  }
  const data = report.data;
  if (data.rows.length === 0 && data.unmeasured.length === 0 && !data.baseline) {
    return <WorkspaceEmptyState icon={Trophy} title={m.results_empty_title()} description={m.results_empty_description()} />;
  }
  const metrics = Object.keys(data.goals);
  const primary = data.primaryMetric;
  const shown = chartMetric && metrics.includes(chartMetric) ? chartMetric : primary;
  const update = (change: Partial<ResultsSettings>) => save.mutate({ ...data.settings, ...change });

  return (
    <div className="absolute inset-0 overflow-auto bg-background @container">
      <div className="flex flex-col gap-5 px-4 py-4">
        <BaselineCard data={data} disabled={save.isPending} onChange={update} onOpenRun={onOpenRun} />
        {data.warnings.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-2 p-0" aria-label={m.results_warnings()}>
            {data.warnings.map((warning, index) => (
              <WarningRow key={index} warning={warning} data={data} onOpenRun={onOpenRun} />
            ))}
          </ul>
        )}
        {shown && (
          <section className="flex flex-col gap-2" aria-labelledby="results-progress">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 id="results-progress" className="m-0 text-sm font-medium text-text">{m.results_progress()}</h3>
              <div className="flex flex-wrap items-center gap-2">
                {metrics.length > 1 && (
                  <div className="w-44 max-w-full">
                  <OptionPicker
                    variant="field"
                    dropDown
                    align="right"
                    title={m.results_metric()}
                    choices={metrics.map((metric) => ({ id: metric, label: metric }))}
                    value={shown}
                    onSelect={setChartMetric}
                  />
                  </div>
                )}
                <GoalToggle
                  metric={shown}
                  goal={data.goals[shown]}
                  disabled={save.isPending}
                  onChange={(goal) => update({ goals: { ...data.settings.goals, [shown]: goal } })}
                />
              </div>
            </div>
            <ProgressChart data={data} metric={shown} onOpenRun={onOpenRun} />
          </section>
        )}
        <section className="flex flex-col gap-2" aria-labelledby="results-ranking">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 id="results-ranking" className="m-0 text-sm font-medium text-text">{m.results_ranking()}</h3>
            {metrics.length > 1 && primary && (
              <div className="w-44 max-w-full">
              <OptionPicker
                variant="field"
                dropDown
                align="right"
                title={m.results_ranked_by()}
                header={m.results_ranked_by()}
                choices={metrics.map((metric) => ({ id: metric, label: metric }))}
                value={primary}
                disabled={save.isPending}
                onSelect={(metric) => update({ primaryMetric: metric })}
              />
              </div>
            )}
          </div>
          <RankingTable data={data} onOpenRun={onOpenRun} />
        </section>
      </div>
    </div>
  );
}

function BaselineCard({ data, disabled, onChange, onOpenRun }: {
  data: ResultsReport;
  disabled: boolean;
  onChange: (change: Partial<ResultsSettings>) => void;
  onOpenRun: (experimentId: string, runId: string) => void;
}) {
  const baseline = data.baseline;
  const choices = [
    { id: AUTO_BASELINE, label: m.results_baseline_auto(), description: m.results_baseline_auto_hint() },
    ...data.rows.map((row) => ({ id: row.runId, label: runLabel(row) })),
  ];
  const selected = data.settings.manualBaseline ? null : data.settings.baselineRunId ?? AUTO_BASELINE;
  const source = baseline?.source === "pinned" ? m.results_baseline_pinned()
    : baseline?.source === "manual" ? m.results_baseline_manual()
    : baseline ? m.results_baseline_root() : null;
  return (
    <section className="flex flex-col gap-3 rounded-lg border border-border bg-surface px-4 py-3" aria-labelledby="results-baseline">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <h3 id="results-baseline" className="m-0 text-xs font-normal text-subtext">{m.results_baseline()}</h3>
          {baseline ? (
            <p className="m-0 flex min-w-0 flex-wrap items-baseline gap-x-2 text-sm">
              {baseline.runId && baseline.experimentId ? (
                <button type="button" className="cursor-pointer truncate border-0 bg-transparent p-0 text-start font-medium text-text hover:underline"
                  onClick={() => onOpenRun(baseline.experimentId!, baseline.runId!)}>{autoDir(baseline.name)}</button>
              ) : <span className="truncate font-medium text-text">{autoDir(baseline.name)}</span>}
              <span className="text-xs text-subtext">{source}</span>
            </p>
          ) : (
            <p className="m-0 text-sm text-subtext">{m.results_baseline_none()}</p>
          )}
        </div>
        <div className="w-64 max-w-full">
          <OptionPicker
            variant="field"
            dropDown
            align="right"
            title={m.results_choose_baseline()}
            header={m.results_choose_baseline()}
            choices={choices}
            value={selected}
            disabled={disabled}
            onSelect={(id) => onChange({ baselineRunId: id === AUTO_BASELINE ? undefined : id, manualBaseline: undefined })}
          />
        </div>
      </div>
      {baseline?.measurement ? (
        <dl className="m-0 flex flex-wrap gap-x-6 gap-y-2">
          {Object.entries(baseline.measurement.metrics).map(([metric, n]) => (
            <div key={metric} className="flex flex-col">
              <dt className={`text-xs ${metric === data.primaryMetric ? "text-text" : "text-subtext"}`}>{ltr(metric)}</dt>
              <dd className="m-0 font-mono text-sm text-text tabular-nums">{value(n)}</dd>
            </div>
          ))}
          <div className="flex flex-col">
            <dt className="text-xs text-subtext">{m.results_data()}</dt>
            <dd className="m-0 text-sm text-text">{ltr(`${baseline.measurement.dataset} / ${baseline.measurement.split}`)}</dd>
          </div>
        </dl>
      ) : baseline && (
        <p className="m-0 text-xs text-subtext">{m.results_baseline_waiting()}</p>
      )}
    </section>
  );
}

function GoalToggle({ metric, goal, disabled, onChange }: { metric: string; goal: Goal | undefined; disabled: boolean; onChange: (goal: Goal) => void }) {
  return (
    <div className={SEGMENTED_CLASS_NAME} role="group" aria-label={m.results_goal_label({ metric: ltr(metric) })}>
      {(["max", "min"] as const).map((option) => (
        <button key={option} type="button" disabled={disabled} className={goal === option ? "active" : ""} aria-pressed={goal === option}
          onClick={() => { if (goal !== option) onChange(option); }}>
          {option === "max" ? <ArrowUp size={12} aria-hidden="true" /> : <ArrowDown size={12} aria-hidden="true" />}
          {option === "max" ? m.results_higher_better() : m.results_lower_better()}
        </button>
      ))}
    </div>
  );
}

function WarningRow({ warning, data, onOpenRun }: { warning: ResultsWarning; data: ResultsReport; onOpenRun: (experimentId: string, runId: string) => void }) {
  const row = "runId" in warning ? data.rows.find((item) => item.runId === warning.runId) : undefined;
  let text: string;
  switch (warning.kind) {
    case "worseThanBaseline":
      text = m.results_warning_worse({ experiment: autoDir(warning.experimentTitle), metric: ltr(warning.metric), value: value(warning.value), baseline: value(warning.baseline) });
      break;
    case "incomparable":
      text = m.results_warning_incomparable({ experiment: autoDir(warning.experimentTitle), data: ltr(`${warning.dataset} / ${warning.split}`) });
      break;
    case "unmeasured": {
      const count = ltr(fmtNumber(warning.count));
      const names = new Intl.ListFormat(getLocale()).format(data.unmeasured.slice(0, 3).map((run) => autoDir(runLabel(run))));
      text = (new Intl.PluralRules(getLocale()).select(warning.count) === "one" ? m.results_warning_unmeasured_one : m.results_warning_unmeasured_other)({ count, runs: names });
      break;
    }
    case "baselineUnmeasured":
      text = m.results_warning_baseline_unmeasured();
      break;
    case "noBaseline":
      text = m.results_warning_no_baseline();
      break;
  }
  const serious = warning.kind === "worseThanBaseline";
  const Icon = warning.kind === "noBaseline" ? Info : AlertTriangle;
  return (
    <li className={`flex flex-wrap items-start gap-2.5 rounded-md border px-3 py-2 text-sm ${serious
      ? "border-accent-red bg-accent-red-subtle text-accent-red"
      : warning.kind === "noBaseline" ? "border-border bg-surface text-subtext" : "border-accent-amber bg-accent-amber-subtle text-accent-amber"}`}>
      <Icon size={16} className="mt-0.5 shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1">{text}</span>
      {row && warning.kind !== "baselineUnmeasured" && (
        <Button size="small" variant="ghost" onClick={() => onOpenRun(row.experimentId, row.runId)}>{m.results_open_run()}</Button>
      )}
    </li>
  );
}

function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    setWidth(element.clientWidth);
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

const CHART_HEIGHT = 200;
const PAD = { top: 12, end: 16, bottom: 24, start: 52 };

function ProgressChart({ data, metric, onOpenRun }: { data: ResultsReport; metric: string; onOpenRun: (experimentId: string, runId: string) => void }) {
  const [ref, width] = useWidth();
  const goal = data.goals[metric];
  const points = data.rows
    .filter((row) => row.verdict !== "incomparable" && Number.isFinite(row.measurement.metrics[metric]))
    .sort((a, b) => a.createdAt - b.createdAt);
  const reference = data.baseline?.measurement?.metrics[metric];
  if (points.length === 0) {
    return <p className="m-0 rounded-lg border border-border-variant px-4 py-6 text-center text-sm text-subtext">{m.results_chart_empty({ metric: ltr(metric) })}</p>;
  }
  const values = points.map((row) => row.measurement.metrics[metric]);
  if (reference != null) values.push(reference);
  let low = Math.min(...values);
  let high = Math.max(...values);
  if (low === high) { low -= Math.abs(low) * 0.1 || 1; high += Math.abs(high) * 0.1 || 1; }
  const margin = (high - low) * 0.08;
  low -= margin;
  high += margin;
  const plotWidth = Math.max(width - PAD.start - PAD.end, 1);
  const plotHeight = CHART_HEIGHT - PAD.top - PAD.bottom;
  const x = (index: number) => PAD.start + (points.length === 1 ? plotWidth / 2 : (index / (points.length - 1)) * plotWidth);
  const y = (n: number) => PAD.top + (1 - (n - low) / (high - low)) * plotHeight;
  let best: number | null = null;
  const bestPath: string[] = [];
  points.forEach((row, index) => {
    const n = row.measurement.metrics[metric];
    if (best == null || gain(n - best, goal) > 0) best = n;
    bestPath.push(index === 0 ? `M${x(index)},${y(best)}` : `H${x(index)} V${y(best)}`);
  });
  const ticks = [low + margin, (low + high) / 2, high - margin];
  const dot = (verdict: Verdict) => verdict === "better" ? "fill-accent-green" : verdict === "worse" ? "fill-accent-red" : verdict === "baseline" ? "fill-subtext" : "fill-accent-blue";
  return (
    <div ref={ref} className="rounded-lg border border-border-variant bg-background px-1 py-2">
      {width > 0 && (
        <svg width={width - 8} height={CHART_HEIGHT} role="img" aria-label={m.results_chart_label({ metric: ltr(metric), count: fmtNumber(points.length) })} className="block overflow-visible">
          {ticks.map((tick) => (
            <g key={tick}>
              <line x1={PAD.start} x2={PAD.start + plotWidth} y1={y(tick)} y2={y(tick)} className="stroke-divider-subtle" />
              <text x={PAD.start - 8} y={y(tick)} textAnchor="end" dominantBaseline="middle" className="fill-subtext text-xs tabular-nums">{value(tick)}</text>
            </g>
          ))}
          {reference != null && (
            <g>
              <line x1={PAD.start} x2={PAD.start + plotWidth} y1={y(reference)} y2={y(reference)} strokeDasharray="5 4" className="stroke-subtext" />
              <text x={PAD.start + plotWidth} y={y(reference) - 5} textAnchor="end" className="fill-subtext text-xs">{m.results_baseline()}</text>
            </g>
          )}
          {points.length > 1 && <path d={bestPath.join(" ")} fill="none" strokeWidth={1.5} strokeDasharray="2 3" className="stroke-accent-green" />}
          {points.length > 1 && <polyline points={points.map((row, index) => `${x(index)},${y(row.measurement.metrics[metric])}`).join(" ")} fill="none" strokeWidth={1.5} className="stroke-accent-blue" />}
          {points.map((row, index) => (
            <circle key={row.runId} cx={x(index)} cy={y(row.measurement.metrics[metric])} r={4.5} className={`${dot(row.verdict)} cursor-pointer stroke-background`} strokeWidth={1.5}
              onClick={() => onOpenRun(row.experimentId, row.runId)}>
              <title>{`${runLabel(row)}: ${value(row.measurement.metrics[metric])}`}</title>
            </circle>
          ))}
          <text x={PAD.start} y={CHART_HEIGHT - 6} className="fill-subtext text-xs">{day(points[0].createdAt)}</text>
          {points.length > 1 && <text x={PAD.start + plotWidth} y={CHART_HEIGHT - 6} textAnchor="end" className="fill-subtext text-xs">{day(points[points.length - 1].createdAt)}</text>}
        </svg>
      )}
      <div className="flex flex-wrap gap-x-4 gap-y-1 px-3 pt-1 text-xs text-subtext" aria-hidden="true">
        <span className="inline-flex items-center gap-1.5"><span className="h-0.5 w-4 bg-accent-blue" />{ltr(metric)}</span>
        {points.length > 1 && <span className="inline-flex items-center gap-1.5"><span className="h-0 w-4 border-t-2 border-dotted border-accent-green" />{m.results_best_so_far()}</span>}
        {reference != null && <span className="inline-flex items-center gap-1.5"><span className="h-0 w-4 border-t border-dashed border-subtext" />{m.results_baseline()}</span>}
      </div>
    </div>
  );
}

function Change({ row, metric, goal, reference }: { row: ResultRow; metric: string; goal: Goal | undefined; reference: number | undefined }) {
  const delta = row.deltas[metric];
  if (row.verdict === "baseline" || delta == null) return <span className="text-subtext">—</span>;
  const better = gain(delta, goal);
  const tone = better > 0 ? "text-accent-green" : better < 0 ? "text-accent-red" : "text-subtext";
  return (
    <span className={`whitespace-nowrap font-mono text-xs tabular-nums ${tone}`}>
      {signed(delta)}
      {reference ? ` (${percent(delta / Math.abs(reference))})` : ""}
    </span>
  );
}

function RankingTable({ data, onOpenRun }: { data: ResultsReport; onOpenRun: (experimentId: string, runId: string) => void }) {
  const primary = data.primaryMetric;
  const others = Object.keys(data.goals).filter((metric) => metric !== primary);
  const reference = data.baseline?.measurement?.metrics ?? {};
  if (data.rows.length === 0) {
    return <p className="m-0 rounded-lg border border-border-variant px-4 py-6 text-center text-sm text-subtext">{m.results_no_measured_runs()}</p>;
  }
  return (
    <div className="overflow-x-auto rounded-lg border border-border">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr className="border-b border-border bg-surface text-xs text-subtext">
            <th scope="col" className="w-10 px-3 py-2 text-start font-normal">{m.results_col_rank()}</th>
            <th scope="col" className="px-3 py-2 text-start font-normal">{m.results_col_run()}</th>
            {primary && <th scope="col" className="px-3 py-2 text-end font-normal">{ltr(primary)}</th>}
            {primary && <th scope="col" className="px-3 py-2 text-end font-normal">{m.results_col_change()}</th>}
            {others.map((metric) => <th key={metric} scope="col" className="px-3 py-2 text-end font-normal">{ltr(metric)}</th>)}
            <th scope="col" className="px-3 py-2 text-start font-normal">{m.results_col_started()}</th>
          </tr>
        </thead>
        <tbody>
          {data.rows.map((row) => {
            const verdict = VERDICTS[row.verdict];
            return (
              <tr key={row.runId} className={`border-b border-border-variant last:border-b-0 hover:bg-hover-faint ${row.verdict === "baseline" ? "bg-surface" : ""}`}>
                <td className="px-3 py-2 align-top text-subtext tabular-nums">{row.rank != null ? fmtNumber(row.rank) : "—"}</td>
                <td className="max-w-72 px-3 py-2 align-top">
                  <button type="button" className="block max-w-full cursor-pointer truncate border-0 bg-transparent p-0 text-start font-medium text-text hover:underline"
                    onClick={() => onOpenRun(row.experimentId, row.runId)}>{autoDir(row.experimentTitle)}</button>
                  <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs text-subtext">
                    <code className="font-mono">{ltr(row.runId.slice(0, 8))}</code>
                    <Badge size="small" variant={verdict.badge}>{verdict.label()}</Badge>
                    {row.verdict === "incomparable" && <span>{ltr(`${row.measurement.dataset} / ${row.measurement.split}`)}</span>}
                  </div>
                </td>
                {primary && (
                  <td className="px-3 py-2 text-end align-top font-mono tabular-nums text-text">
                    {row.measurement.metrics[primary] != null ? value(row.measurement.metrics[primary]) : <span className="text-subtext">—</span>}
                  </td>
                )}
                {primary && <td className="px-3 py-2 text-end align-top"><Change row={row} metric={primary} goal={data.goals[primary]} reference={reference[primary]} /></td>}
                {others.map((metric) => (
                  <td key={metric} className="px-3 py-2 text-end align-top">
                    <div className="font-mono tabular-nums text-text">{row.measurement.metrics[metric] != null ? value(row.measurement.metrics[metric]) : <span className="text-subtext">—</span>}</div>
                    {row.verdict !== "baseline" && row.deltas[metric] != null && <Change row={row} metric={metric} goal={data.goals[metric]} reference={undefined} />}
                  </td>
                ))}
                <td className="whitespace-nowrap px-3 py-2 align-top text-xs text-subtext">{day(row.createdAt)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
