import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react";
import { m } from "../src/paraglide/messages.js";
import { find, renderApp, serveApp, settle, type } from "./render.mjs";

const click = async (element) => {
  await act(async () => element.click());
  await settle();
};

const LINK = "https://drive.google.com/drive/folders/1XVF_efi99XHJERnu-eBKzW5Km0YiNUgc?usp=sharing";
const base = { enabled: false, folderUrl: null, folderId: null, remote: null, rcloneConfig: "/home/me/.config/rclone/rclone.conf", remotes: ["gdrive"], error: null };
const linked = (extra = {}) => ({ ...base, folderUrl: LINK, folderId: "1XVF_efi99XHJERnu-eBKzW5Km0YiNUgc", remote: "gdrive", ...extra });

/** Opens the compute settings with Drive storage answering from `initial`; saves go to `save`. */
async function computeSettings(initial, save) {
  const saved = [];
  serveApp({
    "GET /api/projects": () => ({ projects: [{ id: "p", name: "Project" }] }),
    "GET /api/settings/compute": () => ({ defaultBackend: null, defaultFlavor: null, targets: [] }),
    "GET /api/settings/checkpoints": () => ({ enabled: false }),
    "GET /api/settings/drive-storage": () => initial,
    "POST /api/settings/drive-storage": (init) => {
      const body = JSON.parse(init.body);
      saved.push(body);
      return save(body);
    },
  });
  await renderApp("/projects/p/settings/compute");
  return {
    saved,
    folder: () => document.querySelector('input[type="url"]'),
    toggle: () => document.querySelector(`button[role="switch"][aria-label="${m.settings_drive_storage_title()}"]`),
  };
}

test("Drive storage stays off until a folder is linked and the switch is turned on", async () => {
  let state = base;
  const view = await computeSettings(base, (body) => {
    state = body.folderUrl === "" ? base : body.folderUrl ? linked() : linked({ enabled: body.enabled });
    return state;
  });
  assert.equal(view.toggle().getAttribute("aria-checked"), "false");
  assert.ok(view.toggle().disabled, "nothing to turn on without a folder");

  await type(view.folder(), LINK);
  const saves = [...document.querySelectorAll("button")].filter((b) => b.textContent === m.common_save());
  await click(saves.find((b) => b.closest("form")?.contains(view.folder())));
  assert.deepEqual(view.saved, [{ folderUrl: LINK }]);
  assert.equal(view.toggle().getAttribute("aria-checked"), "false", "linking does not turn it on");
  assert.ok(document.body.textContent.includes(m.settings_drive_storage_hint_off()));

  await click(view.toggle());
  assert.deepEqual(view.saved.at(-1), { enabled: true });
  assert.equal(view.toggle().getAttribute("aria-checked"), "true");

  await click(find("button", m.settings_drive_storage_forget()));
  assert.deepEqual(view.saved.at(-1), { folderUrl: "" });
  assert.equal(view.folder().value, "");
});

test("a folder rclone cannot open shows the server's reason and stays off", async () => {
  const reason = "rclone remote 'gdrive' could not open the folder: directory not found.";
  const view = await computeSettings(linked(), () => Response.json({ error: reason }, { status: 400 }));
  await click(view.toggle());
  assert.ok(document.body.textContent.includes(reason));
  assert.equal(view.toggle().getAttribute("aria-checked"), "false");
});

test("a setup problem, such as rclone missing, is shown", async () => {
  const problem = "rclone is not installed.";
  await computeSettings({ ...base, remotes: [], error: problem }, () => base);
  assert.ok(document.body.textContent.includes(problem));
});
