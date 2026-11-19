import type { ReactNode } from "react";
import { navigateKeyboardRow } from "../keyboardNavigation";
import { cn } from "./ui/cn";
import type { Harness, HarnessSetupCommands } from "../api";
import { m } from "../paraglide/messages.js";
import type { SettingsSection as Tab } from "../workspaceState";

export type SettingsTab = Tab;

// The parts of the Settings page the always-loaded shell needs, kept apart from
// SettingsPage.tsx so that page can load on demand.

type SettingsTabItem = {
  id: Tab;
  label: () => string;
  activeTabs: Tab[];
};

const SETTINGS_TABS: SettingsTabItem[] = [
  { id: "settings", label: m.settings_page_settings, activeTabs: ["settings", "storage"] },
  { id: "agents", label: m.settings_page_harnesses, activeTabs: ["agents"] },
  { id: "compute", label: m.settings_page_compute, activeTabs: ["compute", "openresearch", "instances"] },
  { id: "integrations", label: m.settings_page_integrations, activeTabs: ["integrations"] },
  { id: "environment", label: m.settings_page_environment, activeTabs: ["environment"] },
];

export function SettingsHeader({ active, onSelect, leading }: { active: Tab; onSelect: (tab: Tab) => void; leading: ReactNode }) {
  return (
    <header className="mx-auto flex w-full max-w-readable shrink-0 flex-col gap-2 px-8 pt-2">
      <div className="flex items-center gap-2">
        {leading}
        <h1 tabIndex={-1} className="m-0 text-3xl font-semibold text-text">{m.settings_page_settings()}</h1>
      </div>
      <div
        role="tablist"
        aria-label={m.settings_page_settings()}
        className="flex items-center gap-1 overflow-x-auto border-b border-border py-1 [scrollbar-width:none]"
        onKeyDown={navigateKeyboardRow}
      >
        {SETTINGS_TABS.map((item) => {
          const selected = item.activeTabs.includes(active);
          return (
            <button
              key={item.id}
              type="button"
              role="tab"
              aria-selected={selected}
              tabIndex={selected ? 0 : -1}
              data-keyboard-horizontal-item
              className={cn("relative inline-flex h-9 shrink-0 items-center whitespace-nowrap rounded-md px-3 text-sm hover:text-text focus-visible:outline-2 focus-visible:outline-focus focus-visible:-outline-offset-2", selected ? "font-medium text-text after:absolute after:inset-x-3 after:-bottom-1.25 after:h-0.5 after:bg-text" : "text-subtext")}
              onClick={() => onSelect(item.id)}
            >
              {item.label()}
            </button>
          );
        })}
      </div>
    </header>
  );
}

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

