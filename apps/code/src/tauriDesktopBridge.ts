import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ContextMenuItem,
  DesktopBridge,
  DesktopTheme,
  DesktopUpdateActionResult,
  DesktopUpdateState,
} from "@t3tools/contracts";

type DesktopWindow = Window & { __TAURI_INTERNALS__?: unknown; desktopBridge?: DesktopBridge };
const env = (import.meta as ImportMeta & { env?: Record<string, string | undefined> }).env ?? {};
const MENU_ACTION_EVENT = "desktop:menu-action";
const UPDATE_STATE_EVENT = "desktop:update-state";
const WINDOW_TARGET = "main";

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
    ) => invoke<T | null>("show_context_menu", { items, position }),
    openExternal: (url: string) => invoke<boolean>("open_external", { url }),
    onMenuAction: (listener) => {
      let disposed = false;
      let unlisten: UnlistenFn | null = null;
      void listen<string>(MENU_ACTION_EVENT, (event) => {
        if (typeof event.payload !== "string") return;
        listener(event.payload);
      }, { target: WINDOW_TARGET })
        .then((unsubscribe) => {
          if (disposed) {
            unsubscribe();
            return;
          }
          unlisten = unsubscribe;
        })
        .catch(() => undefined);
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
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
    onUpdateState: (listener) => {
      let disposed = false;
      let unlisten: UnlistenFn | null = null;
      void listen<DesktopUpdateState>(UPDATE_STATE_EVENT, (event) => {
        if (typeof event.payload !== "object" || event.payload === null) return;
        listener(event.payload);
      }, { target: WINDOW_TARGET })
        .then((unsubscribe) => {
          if (disposed) {
            unsubscribe();
            return;
          }
          unlisten = unsubscribe;
        })
        .catch(() => undefined);
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
  } satisfies DesktopBridge;
}
