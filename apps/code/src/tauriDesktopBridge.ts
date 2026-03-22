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
type DesktopWindow = Window & { __TAURI_INTERNALS__?: unknown; desktopBridge?: DesktopBridge };
const env = (import.meta as ImportMeta & { env?: Record<string, string | undefined> }).env ?? {};

const isTauriRuntime =
  typeof window !== "undefined" &&
  ((window as DesktopWindow).__TAURI_INTERNALS__ !== undefined ||
    "__TAURI_INTERNALS__" in (window as DesktopWindow));
let cachedWsUrl: string | null = null;

function createDisabledUpdateState(): DesktopUpdateState {
  return {
    enabled: false,
    status: "disabled",
    currentVersion: env.APP_VERSION ?? "0.0.0",
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

export async function installTauriDesktopBridge(): Promise<void> {
  if (typeof window === "undefined" || !isTauriRuntime || (window as DesktopWindow).desktopBridge) {
    return;
  }

  cachedWsUrl = await invoke<string | null>("get_ws_url").catch(() => null);
  (window as DesktopWindow).desktopBridge = {
    getWsUrl: () => cachedWsUrl || env.VITE_WS_URL || null,
    pickFolder: () => invoke<string | null>("pick_folder"),
    confirm: (message: string) => invoke<boolean>("confirm_dialog", { message }),
    setTheme: (theme: DesktopTheme) => invoke<void>("set_theme", { theme }),
    showContextMenu: <T extends string>(
      items: readonly ContextMenuItem<T>[],
      position?: { x: number; y: number },
    ) => showContextMenuFallback(items, position),
    openExternal: (url: string) => invoke<boolean>("open_external", { url }),
    onMenuAction: () => noopUnsubscribe,
    getUpdateState: () =>
      invoke<DesktopUpdateState>("get_update_state").catch(() => createDisabledUpdateState()),
    downloadUpdate: () =>
      invoke<DesktopUpdateActionResult>("download_update").catch(async () => ({
        accepted: false,
        completed: false,
        state: createDisabledUpdateState(),
      })),
    installUpdate: () =>
      invoke<DesktopUpdateActionResult>("install_update").catch(async () => ({
        accepted: false,
        completed: false,
        state: createDisabledUpdateState(),
      })),
    onUpdateState: () => noopUnsubscribe,
  } satisfies DesktopBridge;
}
