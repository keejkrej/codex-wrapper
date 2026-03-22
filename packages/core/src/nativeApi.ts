import type { NativeApi } from "@t3tools/contracts";

import { createWsNativeApi } from "./wsNativeApi";

let cachedApi: NativeApi | undefined;
type NativeWindow = Window & { nativeApi?: NativeApi };

export function readNativeApi(): NativeApi | undefined {
  if (typeof window === "undefined") return undefined;
  if (cachedApi) return cachedApi;

  const nativeWindow = window as NativeWindow;
  if (nativeWindow.nativeApi) {
    cachedApi = nativeWindow.nativeApi;
    return cachedApi;
  }

  cachedApi = createWsNativeApi();
  return cachedApi;
}

export function ensureNativeApi(): NativeApi {
  const api = readNativeApi();
  if (!api) {
    throw new Error("Native API not found");
  }
  return api;
}
