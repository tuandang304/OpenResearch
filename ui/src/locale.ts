import { useSyncExternalStore } from "react";
import {
  getLocale,
  setLocale as setParaglideLocale,
  type Locale,
} from "./paraglide/runtime.js";
import { loadLocaleMessages } from "virtual:paraglide-locales";

const listeners = new Set<() => void>();
let pending: Locale | null = null;

export function setLocale(next: Locale): void {
  if (next === (pending ?? getLocale())) return;
  pending = next;
  // Builds ship only the base locale up front, so switch once the new one has loaded.
  void loadLocaleMessages(next).catch(() => {}).then(() => {
    if (pending !== next) return;
    pending = null;
    void setParaglideLocale(next, { reload: false });
    document.documentElement.lang = next;
    for (const listener of listeners) listener();
  });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useLocale(): Locale {
  return useSyncExternalStore(subscribe, getLocale, getLocale);
}
