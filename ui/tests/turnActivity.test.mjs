import assert from "node:assert/strict";
import test from "node:test";
import { currentTurnMessages, fmtClock, outputTail, turnActivity } from "../src/turnActivity.ts";

const user = (id, createdAt) => ({ id, role: "user", parts: [{ id: `${id}-text`, type: "text", text: "hi" }], createdAt });
const reply = (id, ...parts) => ({ id, role: "assistant", parts, createdAt: 50 });
const tool = (id, status, output) => ({ id, type: "tool", tool: "bash", state: { status, input: { command: "ls" }, output } });

test("current turn starts after the newest user message", () => {
  const messages = [user("u1", 1), reply("a1"), user("u2", 10), reply("a2"), reply("a3")];
  assert.deepEqual(currentTurnMessages(messages).map((message) => message.id), ["a2", "a3"]);
  assert.equal(turnActivity(messages).startedAt, 10);
});

test("phase follows the newest streamed part", () => {
  assert.equal(turnActivity([user("u", 0)]).phase, "starting");
  assert.equal(turnActivity([user("u", 0), reply("a", { id: "r", type: "reasoning", text: "" })]).phase, "thinking");
  const running = turnActivity([user("u", 0), reply("a", tool("t1", "completed"), tool("t2", "running"))]);
  assert.equal(running.phase, "tool");
  assert.equal(running.current?.id, "t2");
  assert.equal(running.steps.length, 2);
  assert.equal(turnActivity([user("u", 0), reply("a", tool("t1", "completed"), { id: "x", type: "text", text: "Done" })]).phase, "writing");
  assert.equal(turnActivity([user("u", 0), reply("a", tool("t1", "running"))], true).phase, "waiting");
});

test("signature changes when output streams", () => {
  const before = turnActivity([user("u", 0), reply("a", tool("t", "running", "a"))]).signature;
  const after = turnActivity([user("u", 0), reply("a", tool("t", "running", "ab"))]).signature;
  assert.notEqual(before, after);
});

test("output tail keeps the last lines and the final progress frame", () => {
  assert.equal(outputTail("one\ntwo\n\nthree\nfour\n"), "two\nthree\nfour");
  assert.equal(outputTail("10%\r50%\r100%"), "100%");
  assert.equal(outputTail(undefined), "");
});

test("clock formatting", () => {
  assert.equal(fmtClock(7_000), "0:07");
  assert.equal(fmtClock(133_000), "2:13");
  assert.equal(fmtClock(3_729_000), "1:02:09");
});
