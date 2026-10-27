import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronUp, CircleX, Hourglass, ListChecks, TriangleAlert } from "lucide-react";
import type { ChatMessage, ChatPart } from "../api";
import { m } from "../paraglide/messages.js";
import { useLocale } from "../locale";
import { fmtClock, outputTail, TURN_STALL_MS, turnActivity } from "../turnActivity";
import { Button, Spinner } from "./ui";
import { cn } from "./ui/cn";

export interface StepDescription {
  icon: React.ReactNode;
  /** Present-tense verb while running ("Running…"). */
  verb: string;
  /** Past-tense summary once done ("Ran npm test"). */
  label: string;
  /** The command, path or query the step works on. */
  detail: string | null;
}

interface StepTiming {
  start: number;
  end?: number;
}

/** Live status of the running turn, docked above the composer: what the
 * harness is doing now, how long it has taken, the tail of streamed output,
 * and a warning once it has gone quiet for a while. */
export function HarnessActivity({
  messages,
  awaitingInput,
  harnessLabel,
  describe,
  onStop,
}: {
  /** The branch on screen, oldest first. */
  messages: ChatMessage[];
  awaitingInput: boolean;
  harnessLabel: string;
  describe: (part: ChatPart) => StepDescription;
  onStop?: () => void;
}) {
  useLocale();
  const activity = useMemo(() => turnActivity(messages, awaitingInput), [messages, awaitingInput]);
  const [now, setNow] = useState(() => Date.now());
  const [expanded, setExpanded] = useState(false);
  const mountedAt = useRef(Date.now());
  const lastChange = useRef({ signature: activity.signature, at: Date.now() });
  // Parts carry no timestamps, so step times are what this view observed.
  const timings = useRef(new Map<string, StepTiming>());
  const timelineRef = useRef<HTMLOListElement>(null);
  const timelineId = useId();

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    const at = Date.now();
    if (lastChange.current.signature !== activity.signature) lastChange.current = { signature: activity.signature, at };
    const seen = timings.current;
    for (const part of activity.steps) {
      const timing = seen.get(part.id);
      const running = part.state?.status === "running";
      if (!timing) {
        // Steps already finished before this view mounted have no known duration.
        if (running) seen.set(part.id, { start: at });
      } else if (!running && timing.end === undefined) {
        timing.end = at;
      }
    }
  }, [activity]);

  // Keep the newest step in view as the timeline grows.
  useLayoutEffect(() => {
    const list = timelineRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [expanded, activity.steps.length]);

  const startedAt = activity.startedAt ?? mountedAt.current;
  const quietFor = now - lastChange.current.at;
  const stalled = activity.phase !== "waiting" && quietFor >= TURN_STALL_MS;
  const current = activity.current;
  const currentDescription = current ? describe(current) : null;
  const currentTiming = current ? timings.current.get(current.id) : undefined;
  const tail = current ? outputTail(current.state?.output) : "";

  const headline = activity.phase === "waiting" ? m.harness_activity_waiting()
    : activity.phase === "tool" && currentDescription ? currentDescription.verb
    : activity.phase === "writing" ? m.harness_activity_writing({ harness: harnessLabel })
    : activity.phase === "thinking" ? m.harness_activity_thinking({ harness: harnessLabel })
    : m.harness_activity_starting({ harness: harnessLabel });

  return (
    <div
      className={cn(
        "harness-activity relative -mb-px rounded-t-lg border border-b-0 bg-panel text-sm text-text",
        stalled ? "border-accent-amber" : "border-border",
      )}
      aria-label={m.harness_activity_label()}
    >
      <div className="flex h-9 min-w-0 items-center gap-2 px-3" role="status" aria-live="polite">
        {activity.phase === "waiting" ? (
          <Hourglass size={14} className="shrink-0 text-accent-blue" aria-hidden="true" />
        ) : stalled ? (
          <TriangleAlert size={14} className="shrink-0 text-accent-amber" aria-hidden="true" />
        ) : (
          <Spinner />
        )}
        <span className="min-w-0 truncate font-medium">{headline}</span>
        {/* Takes only the space left, so the headline never gives way to a long command. */}
        <span className="flex min-w-0 flex-1 basis-0 items-center gap-2">
          {currentDescription?.detail && (
            <code className="min-w-0 truncate rounded-sm bg-surface px-1.5 py-px font-mono text-xs text-subtext" title={currentDescription.detail}>
              {currentDescription.detail}
            </code>
          )}
          {currentTiming && now - currentTiming.start >= 1000 && (
            <span className="shrink-0 text-xs tabular-nums text-muted" title={m.harness_activity_step_time()}>
              {fmtClock(now - currentTiming.start)}
            </span>
          )}
        </span>
        <span className="flex shrink-0 items-center gap-1 text-subtext">
          <span className="px-1 tabular-nums" title={m.harness_activity_elapsed()}>{fmtClock(now - startedAt)}</span>
          {activity.steps.length > 0 && (
            <button
              type="button"
              className="harness-activity-toggle inline-flex h-6 items-center gap-1 rounded-sm px-1.5 cursor-pointer hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-focus"
              aria-expanded={expanded}
              aria-controls={timelineId}
              onClick={() => setExpanded((value) => !value)}
            >
              <ListChecks size={14} aria-hidden="true" />
              <span className="tabular-nums">{m.harness_activity_steps({ count: String(activity.steps.length) })}</span>
              <ChevronUp size={14} className={cn("transition-transform duration-120 ease-standard motion-reduce:transition-none", !expanded && "rotate-180")} aria-hidden="true" />
            </button>
          )}
        </span>
      </div>

      {expanded && (
        <ol
          id={timelineId}
          ref={timelineRef}
          className="harness-activity-timeline m-0 flex max-h-48 list-none flex-col overflow-y-auto border-t border-border-variant px-3 py-1.5 ps-3"
        >
          {activity.steps.map((part) => {
            const description = describe(part);
            const status = part.state?.status;
            const timing = timings.current.get(part.id);
            const duration = timing ? (timing.end ?? now) - timing.start : null;
            return (
              <li key={part.id} className="flex h-6 min-w-0 items-center gap-2">
                <span className="flex w-4 shrink-0 justify-center">
                  {status === "running" ? <Spinner className="size-3" />
                    : status === "error" ? <CircleX size={14} className="text-accent-red" aria-hidden="true" />
                    : <Check size={14} className="text-accent-green" aria-hidden="true" />}
                </span>
                <span className="flex shrink-0 text-muted [&_svg]:size-3.5" aria-hidden="true">{description.icon}</span>
                <span className={cn("min-w-0 flex-1 truncate", status === "error" ? "text-accent-red" : "text-subtext")} title={description.detail ?? description.label}>
                  {status === "error" && <span className="sr-only">{m.chat_panel_failed()} </span>}
                  {status === "running" ? description.verb : description.label}
                  {status === "running" && description.detail && <span className="ms-1 font-mono text-xs">{description.detail}</span>}
                </span>
                {duration !== null && duration >= 1000 && <span className="shrink-0 text-xs tabular-nums text-muted">{fmtClock(duration)}</span>}
              </li>
            );
          })}
        </ol>
      )}

      {tail && (
        <div className="px-3 pb-2">
          <pre className="harness-activity-output m-0 max-h-15 overflow-hidden rounded-sm bg-background px-2 py-1 font-mono text-xs leading-4 text-subtext whitespace-pre-wrap wrap-anywhere">
            {tail}
          </pre>
        </div>
      )}

      {stalled && (
        <div className="flex items-center gap-2 border-t border-border-variant px-3 py-1.5 text-subtext">
          <span className="min-w-0 flex-1 text-xs leading-5">{m.harness_activity_quiet({ time: fmtClock(quietFor) })}</span>
          {onStop && (
            <Button type="button" size="small" onClick={onStop}>{m.chat_panel_stop()}</Button>
          )}
        </div>
      )}
    </div>
  );
}
