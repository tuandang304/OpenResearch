import { useEffect, useState } from "react";
import { fmtDuration, fmtNumber, type ChatMessage } from "../api";
import { m } from "../paraglide/messages.js";
import { useLocale } from "../locale";

/** The "Worked for 1m 12s" label on a turn's work disclosure. While the turn
 * streams it ticks every second; the clock lives in this leaf so the tick
 * repaints one label rather than the whole turn's tool rows and answer. */
export function WorkDuration({ messages, streaming }: {
  /** The turn's assistant messages, oldest first. */
  messages: ChatMessage[];
  streaming: boolean;
}) {
  useLocale();
  const message = messages[messages.length - 1];
  const [, tick] = useState(0);
  useEffect(() => {
    if (!streaming || message.completedAt != null) return;
    const timer = window.setInterval(() => tick((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, [streaming, message.completedAt]);
  const elapsed = messages.reduce<number | null>((total, turn) => {
    const end = turn.completedAt ?? (streaming && turn.id === message.id ? Date.now() : null);
    return end === null ? total : (total ?? 0) + Math.max(0, end - turn.createdAt);
  }, null);
  const duration = elapsed === null ? null : elapsed < 60_000 || elapsed >= 3_600_000
    ? fmtDuration(elapsed)
    : m.chat_work_duration({ minutes: fmtNumber(Math.floor(elapsed / 60_000)), seconds: fmtNumber(Math.floor(elapsed / 1000) % 60) });
  return <span>{duration === null ? m.chat_work_details() : m.chat_worked_for({ duration })}</span>;
}
