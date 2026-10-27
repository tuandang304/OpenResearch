import type { ChatMessage, ChatPart } from "./api.ts";
import { isTurnStatusPart, partIsVisible } from "./chatRendering.ts";

/** Quiet time after which a busy turn reads as possibly stuck. */
export const TURN_STALL_MS = 90_000;

export type TurnPhase = "starting" | "thinking" | "tool" | "writing" | "waiting";

export interface TurnActivity {
  /** When the user sent the turn (or the first reply arrived), in ms. */
  startedAt: number | null;
  /** Tool calls this turn made, oldest first. */
  steps: ChatPart[];
  /** The newest tool call still running. */
  current: ChatPart | null;
  phase: TurnPhase;
  /** Changes whenever the harness streams anything new. */
  signature: string;
}

/** The assistant messages answering the newest user message. */
export function currentTurnMessages(messages: readonly ChatMessage[]): ChatMessage[] {
  let index = messages.length;
  while (index > 0 && messages[index - 1].role === "assistant") index--;
  return messages.slice(index);
}

export function turnActivity(messages: readonly ChatMessage[], awaitingInput = false): TurnActivity {
  const replies = currentTurnMessages(messages);
  const prompt = replies.length < messages.length ? messages[messages.length - replies.length - 1] : null;
  const startedAt = prompt?.createdAt ?? replies[0]?.createdAt ?? null;
  const steps: ChatPart[] = [];
  const signature: (string | number)[] = [];
  let tail: ChatPart | null = null;
  for (const message of replies) {
    signature.push(message.id, message.parts.length);
    for (const part of message.parts) {
      signature.push(part.text?.length ?? 0, part.state?.status ?? "", part.state?.output?.length ?? 0, part.children?.length ?? 0);
      if (part.type === "steer" || isTurnStatusPart(part)) continue;
      if (part.type === "tool" && partIsVisible(part)) steps.push(part);
      if (partIsVisible(part) || part.type === "reasoning") tail = part;
    }
  }
  let current: ChatPart | null = null;
  for (const part of steps) if (part.state?.status === "running") current = part;
  const phase: TurnPhase = awaitingInput ? "waiting"
    : current ? "tool"
    : tail?.type === "text" && tail.text ? "writing"
    : tail ? "thinking"
    : "starting";
  return { startedAt, steps, current, phase, signature: signature.join(":") };
}

/** The last few non-empty lines of streamed tool output. */
export function outputTail(output: string | undefined, maxLines = 3, maxChars = 400): string {
  if (!output) return "";
  const recent = output.length > maxChars * 4 ? output.slice(-maxChars * 4) : output;
  // Carriage returns redraw progress bars in place; keep the final frame.
  const lines = recent.split("\n").map((line) => line.split("\r").at(-1)?.trimEnd() ?? "").filter(Boolean);
  const text = lines.slice(-maxLines).join("\n");
  return text.length > maxChars ? `…${text.slice(-maxChars)}` : text;
}

/** Clock-style elapsed time: 0:07, 2:13, 1:02:09. */
export function fmtClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}
