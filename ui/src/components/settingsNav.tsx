import { Cpu, Settings, SquareTerminal } from "lucide-react";
import type { Harness, HarnessSetupCommands } from "../api";
import { m } from "../paraglide/messages.js";
import type { SettingsSection as Tab } from "../workspaceState";

// The parts of the Settings page the always-loaded shell needs, kept apart from
// SettingsPage.tsx so that page can load on demand.

type SettingsNavItem = {
  id: Tab;
  label: () => string;
  icon: React.ReactNode;
  activeTabs: Tab[];
};

export const SETTINGS_SECTIONS: Tab[] = ["projects", "harnesses", "context", "storage"];

/** Primary rail entries. Configuration sections share the Settings entry. */
export const SETTINGS_NAV: SettingsNavItem[] = [
  {
    id: "compute",
    label: m.settings_page_compute,
    icon: <Cpu size={16} />,
    activeTabs: ["compute", "instances"],
  },
  { id: "environment", label: m.settings_page_environment, icon: <SquareTerminal size={16} />, activeTabs: ["environment"] },
  {
    id: "settings",
    label: m.settings_page_settings,
    icon: <Settings size={16} />,
    activeTabs: ["settings", ...SETTINGS_SECTIONS],
  },
];

/** A note command that is one of the harness's setup commands runs through the
 * setup route, which owns install/login/update semantics (telemetry, OpenCode's
 * isolated store, verification); `shell` keeps the terminal open afterwards. */
export function harnessSetupPath(h: Harness, setup: HarnessSetupCommands | undefined, command: string) {
  if (!setup) return undefined;
  // Install notes quote the vendor one-liner (`curl … | bash`) while the shown
  // setup command fetches to a temp file first, so exact equality would drop the
  // play button from every install note. The shared bootstrap URL identifies it.
  const installsSameSource = setup.installUrl !== undefined && command.includes(setup.installUrl);
  const action = command === setup.login ? "login" : command === setup.install ? "install" : command === setup.update ? "update" : installsSameSource ? "install" : null;
  if (!action) return undefined;
  if (action !== "install" && (!h.installed || h.installBroken)) return undefined;
  return `/api/harnesses/setup?${new URLSearchParams({ harness: h.id, action, shell: "true" })}`;
}

