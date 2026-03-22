import { describe, expect, it, vi } from "vitest";

import { subscribeDesktopMenuActions } from "./desktopMenuActions";

describe("chat route desktop menu actions", () => {
  it("routes open-settings to the provided callback", () => {
    let capturedHandler: ((action: string) => void) | undefined;
    const unlisten = vi.fn();
    const onOpenSettings = vi.fn();

    const unsubscribe = subscribeDesktopMenuActions(
      {
        getWsUrl: () => null,
        pickFolder: async () => null,
        confirm: async () => false,
        setTheme: async () => undefined,
        showContextMenu: async () => null,
        openExternal: async () => false,
        onMenuAction: (listener) => {
          capturedHandler = listener;
          return unlisten;
        },
        getUpdateState: async () => ({
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
          message: null,
          errorContext: null,
          canRetry: false,
        }),
        downloadUpdate: async () => ({
          accepted: false,
          completed: false,
          state: {
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
            message: null,
            errorContext: null,
            canRetry: false,
          },
        }),
        installUpdate: async () => ({
          accepted: false,
          completed: false,
          state: {
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
            message: null,
            errorContext: null,
            canRetry: false,
          },
        }),
        onUpdateState: () => () => undefined,
      },
      onOpenSettings,
    );

    if (capturedHandler) {
      capturedHandler("ignored");
    }
    expect(onOpenSettings).not.toHaveBeenCalled();

    if (capturedHandler) {
      capturedHandler("open-settings");
    }
    expect(onOpenSettings).toHaveBeenCalledTimes(1);

    unsubscribe?.();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
