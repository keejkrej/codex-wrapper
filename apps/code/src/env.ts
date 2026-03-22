const isTauriRuntime =
  typeof window !== "undefined" &&
  (window.__TAURI_INTERNALS__ !== undefined || "__TAURI_INTERNALS__" in window);

/**
 * True when running inside a desktop shell bridge, false in a regular browser.
 * This preserves the existing desktop-only branches while broadening them
 * from Electron preload detection to Tauri as well.
 */
export const isElectron =
  typeof window !== "undefined" &&
  (window.desktopBridge !== undefined || window.nativeApi !== undefined || isTauriRuntime);
