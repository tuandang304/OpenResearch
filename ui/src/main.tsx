import { AppMenuBridge } from "./components/AppMenuBridge";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";
import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "./queries/client";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { HotkeysProvider } from "@tanstack/react-hotkeys";
import { appHotkeys } from "./hotkeys";
import { router } from "./router";
import { DialogManager } from "./dialog";
import { Toaster } from "./components/ui";
import { DesktopZoomControls } from "./components/DesktopZoomControls";
import { WindowControls } from "./components/WindowControls";
import { WINDOWS_TITLEBAR } from "./desktopTitlebar";
import { getLocale } from "./paraglide/runtime.js";
import { loadLocaleMessages } from "virtual:paraglide-locales";
import "./tailwind.css";

// openresearch:// links opened in the desktop app; see the Opened handler in src/commands/app.rs.
window.addEventListener("orx:open-path", (event) => {
  if (event instanceof CustomEvent && typeof event.detail === "string") void router.navigate({ href: event.detail });
});

const locale = getLocale();
document.documentElement.lang = locale;
document.documentElement.dir = "ltr";

// Builds ship only the base locale up front; load the active one before the first render.
void loadLocaleMessages(locale).catch(() => {}).then(() => createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <HotkeysProvider appHotkeys={appHotkeys}>
        {"__ORX_DESKTOP__" in window && <AppMenuBridge />}
        <RouterProvider router={router} />
        <DialogManager />
        {WINDOWS_TITLEBAR && <WindowControls />}
      </HotkeysProvider>
      {import.meta.env.DEV && <ReactQueryDevtools buttonPosition="bottom-right" />}
    </QueryClientProvider>
    <Toaster />
    {"__ORX_DESKTOP__" in window && <DesktopZoomControls />}
  </StrictMode>,
));
