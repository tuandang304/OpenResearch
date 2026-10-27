/// <reference types="vite/client" />

interface Window {
  /** wry's channel to the desktop app; absent in a browser. */
  ipc?: { postMessage(message: string): void };
}

/** Non-base locale messages, split out of the entry chunk by paraglideLocaleSplit.ts. */
declare module "virtual:paraglide-locales" {
  /** Resolves once `locale`'s messages are loaded; until then `m.*()` falls back to the base locale. */
  export function loadLocaleMessages(locale: string): Promise<void>;
}
