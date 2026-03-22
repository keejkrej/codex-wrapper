import type { DesktopBridge, NativeApi } from "@t3tools/contracts";

type DesktopWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
  desktopBridge?: DesktopBridge;
  nativeApi?: NativeApi;
};

const desktopWindow = typeof window === "undefined" ? undefined : (window as DesktopWindow);
const isTauriRuntime =
  desktopWindow !== undefined &&
  (desktopWindow.__TAURI_INTERNALS__ !== undefined || "__TAURI_INTERNALS__" in desktopWindow);

/**
 * True when running inside a desktop shell bridge, false in a regular browser.
 * This preserves the existing desktop-only branches while broadening them
 * from Electron preload detection to Tauri as well.
 */
export const isElectron =
  desktopWindow !== undefined &&
  (desktopWindow.desktopBridge !== undefined ||
    desktopWindow.nativeApi !== undefined ||
    isTauriRuntime);
