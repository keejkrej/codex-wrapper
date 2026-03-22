import { invoke } from "@tauri-apps/api/core";
import type {
  ContextMenuItem,
  DesktopBridge,
  DesktopTheme,
  DesktopUpdateActionResult,
  DesktopUpdateState,
} from "@t3tools/contracts";

import { showContextMenuFallback } from "./contextMenuFallback";

const noopUnsubscribe = () => {};

const isTauriRuntime =
  typeof window !== "undefined" &&
  (window.__TAURI_INTERNALS__ !== undefined || "__TAURI_INTERNALS__" in window);

function createDisabledUpdateState(): DesktopUpdateState {
  return {
    enabled: false,
    status: "disabled",
    currentVersion: import.meta.env.APP_VERSION,
    hostArch: "other",
    appArch: "other",
    runningUnderArm64Translation: false,
    availableVersion: null,
    downloadedVersion: null,
    downloadPercent: null,
    checkedAt: null,
    message: "Desktop update support has not been ported to the Tauri shell yet.",
    errorContext: null,
    canRetry: false,
  };
}

export function installTauriDesktopBridge(): void {
  if (typeof window === "undefined" || !isTauriRuntime || window.desktopBridge) {
    return;
  }

  const disabledUpdateState = createDisabledUpdateState();
  const buildDisabledUpdateAction = async (): Promise<DesktopUpdateActionResult> => ({
    accepted: false,
    completed: false,
    state: disabledUpdateState,
  });

  window.desktopBridge = {
    getWsUrl: () => import.meta.env.VITE_WS_URL || null,
    pickFolder: () => invoke<string | null>("pick_folder"),
    confirm: (message: string) => invoke<boolean>("confirm_dialog", { message }),
    setTheme: (theme: DesktopTheme) => invoke<void>("set_theme", { theme }),
    showContextMenu: <T extends string>(
      items: readonly ContextMenuItem<T>[],
      position?: { x: number; y: number },
    ) => showContextMenuFallback(items, position),
    openExternal: (url: string) => invoke<boolean>("open_external", { url }),
    onMenuAction: () => noopUnsubscribe,
    getUpdateState: async () => disabledUpdateState,
    downloadUpdate: buildDisabledUpdateAction,
    installUpdate: buildDisabledUpdateAction,
    onUpdateState: () => noopUnsubscribe,
  } satisfies DesktopBridge;
}
