import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react";
import { m } from "../src/paraglide/messages.js";
import { find, renderApp, serveApp, settle, type } from "./render.mjs";

const click = async (element) => {
  await act(async () => element.click());
  await settle();
};

/** Opens the compute settings with checkpoint storage answering from `initial`; saves go to `save`. */
async function computeSettings(initial, save) {
  const saved = [];
  serveApp({
    "GET /api/projects": () => ({ projects: [{ id: "p", name: "Project" }] }),
    "GET /api/settings/compute": () => ({ defaultBackend: null, defaultFlavor: null, targets: [] }),
    "GET /api/settings/checkpoints": () => initial,
    "POST /api/settings/checkpoints": (init) => {
      const body = JSON.parse(init.body);
      saved.push(body);
      return save(body);
    },
  });
  await renderApp("/projects/p/settings/compute");
  return {
    saved,
    remote: () => document.querySelector('input[placeholder="gdrive:openresearch"]'),
    save: () => find("button", m.common_save()),
  };
}

const on = (remote, extra = {}) => ({ enabled: true, remote, intervalMinutes: 10, rcloneConfig: "/home/me/.config/rclone/rclone.conf", error: null, ...extra });

test("saving a remote turns checkpoint storage on", async () => {
  const view = await computeSettings({ enabled: false }, (body) => on(body.remote));
  assert.ok(document.body.textContent.includes(m.settings_checkpoints_off()));
  await type(view.remote(), "gdrive:openresearch");
  await click(view.save());
  assert.deepEqual(view.saved, [{ remote: "gdrive:openresearch" }]);
  assert.ok(document.body.textContent.includes(m.settings_checkpoints_on()));

  await click(find("button", m.settings_checkpoints_turn_off()));
  assert.equal(view.saved.at(-1).remote, "");
});

test("a remote rclone does not know shows the server's reason", async () => {
  const reason = "rclone has no remote named 'gdrive'. Add it with `rclone config`";
  const view = await computeSettings({ enabled: false }, () => Response.json({ error: reason }, { status: 400 }));
  await type(view.remote(), "gdrive:openresearch");
  await click(view.save());
  assert.ok(document.body.textContent.includes(reason));
  assert.ok(document.body.textContent.includes(m.settings_checkpoints_off()));
});

test("a saved remote that runs cannot use says why", async () => {
  const problem = "Could not read the rclone config /home/me/.config/rclone/rclone.conf";
  await computeSettings(on("gdrive:openresearch", { error: problem }), () => on("gdrive:openresearch"));
  assert.ok(document.body.textContent.includes(problem));
});
