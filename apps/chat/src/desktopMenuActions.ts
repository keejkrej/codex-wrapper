import type { DesktopBridge } from "@t3tools/contracts";

export function subscribeDesktopMenuActions(
  bridge: DesktopBridge | undefined,
  onOpenSettings: () => void,
): (() => void) | undefined {
  const onMenuAction = bridge?.onMenuAction;
  if (typeof onMenuAction !== "function") {
    return undefined;
  }

  return onMenuAction((action) => {
    if (action !== "open-settings") return;
    onOpenSettings();
  });
}
