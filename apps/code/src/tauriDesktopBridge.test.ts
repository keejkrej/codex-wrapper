import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn<(command: string, payload?: unknown) => Promise<unknown>>();
const listenMock = vi.fn<
  (
    event: string,
    handler: (event: { payload: unknown }) => void,
    options?: { target?: string },
  ) => Promise<() => void>
>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

type DesktopWindow = Window & typeof globalThis & { __TAURI_INTERNALS__?: unknown; desktopBridge?: unknown };

function getWindowForTest(): DesktopWindow {
  const testGlobal = globalThis as typeof globalThis & {
    window?: DesktopWindow;
  };
  if (!testGlobal.window) {
    testGlobal.window = {} as DesktopWindow;
  }
  return testGlobal.window;
}

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  const testWindow = getWindowForTest();
  testWindow.__TAURI_INTERNALS__ = {};
  Reflect.deleteProperty(testWindow, "desktopBridge");
  invokeMock.mockImplementation(async (command) => {
    if (command === "get_ws_url") {
      return "ws://127.0.0.1:3773/ws";
    }
    if (command === "show_context_menu") {
      return "delete";
    }
    if (command === "get_update_state") {
      return {
        enabled: false,
        status: "disabled",
        currentVersion: "0.0.0",
        hostArch: "other",
        appArch: "other",
        runningUnderArm64Translation: false,
        availableVersion: null,
        downloadedVersion: null,
        downloadPercent: null,
        checkedAt: null,
        message: "disabled",
        errorContext: null,
        canRetry: false,
      };
    }
    return null;
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("tauriDesktopBridge", () => {
  it("uses the native context menu command in tauri mode", async () => {
    const { installTauriDesktopBridge } = await import("./tauriDesktopBridge");

    await installTauriDesktopBridge();
    const bridge = (getWindowForTest().desktopBridge as {
      showContextMenu: <T extends string>(
        items: readonly { id: T; label: string; destructive?: boolean }[],
        position?: { x: number; y: number },
      ) => Promise<T | null>;
    })!;

    const result = await bridge.showContextMenu(
      [{ id: "delete", label: "Delete", destructive: true }],
      { x: 10, y: 20 },
    );

    expect(result).toBe("delete");
    expect(invokeMock).toHaveBeenCalledWith("show_context_menu", {
      items: [{ id: "delete", label: "Delete", destructive: true }],
      position: { x: 10, y: 20 },
    });
  });

  it("subscribes and unsubscribes menu actions through tauri events", async () => {
    let handler: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    listenMock.mockImplementation(async (event, callback) => {
      if (event === "desktop:menu-action") {
        handler = callback;
      }
      return unlisten;
    });

    const { installTauriDesktopBridge } = await import("./tauriDesktopBridge");
    await installTauriDesktopBridge();
    const bridge = getWindowForTest().desktopBridge as {
      onMenuAction: (listener: (action: string) => void) => () => void;
    };
    const listener = vi.fn();

    const unsubscribe = bridge.onMenuAction(listener);
    await Promise.resolve();
    await Promise.resolve();
    if (handler) {
      handler({ payload: "open-settings" });
    }

    expect(listenMock).toHaveBeenCalledWith(
      "desktop:menu-action",
      expect.any(Function),
      { target: "main" },
    );
    expect(listener).toHaveBeenCalledWith("open-settings");

    unsubscribe();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("subscribes and unsubscribes update state through tauri events", async () => {
    let handler: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    listenMock.mockImplementation(async (event, callback) => {
      if (event === "desktop:update-state") {
        handler = callback;
      }
      return unlisten;
    });

    const { installTauriDesktopBridge } = await import("./tauriDesktopBridge");
    await installTauriDesktopBridge();
    const bridge = getWindowForTest().desktopBridge as {
      onUpdateState: (listener: (state: unknown) => void) => () => void;
    };
    const listener = vi.fn();

    const unsubscribe = bridge.onUpdateState(listener);
    await Promise.resolve();
    await Promise.resolve();
    if (handler) {
      handler({
        payload: {
          enabled: false,
          status: "disabled",
          currentVersion: "0.0.0",
          hostArch: "other",
          appArch: "other",
          runningUnderArm64Translation: false,
          availableVersion: null,
          downloadedVersion: null,
          downloadPercent: null,
          checkedAt: null,
          message: "disabled",
          errorContext: null,
          canRetry: false,
        },
      });
    }

    expect(listenMock).toHaveBeenCalledWith(
      "desktop:update-state",
      expect.any(Function),
      { target: "main" },
    );
    expect(listener).toHaveBeenCalledWith(
      expect.objectContaining({ status: "disabled" }),
    );

    unsubscribe();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
