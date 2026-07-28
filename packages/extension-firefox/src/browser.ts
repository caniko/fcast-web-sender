import { initialState, reduce, reduceBridgeEvent, type ExtensionState } from "@fcast/extension-core";
import type { BridgeEvent, BridgeMethod, BridgeResponse, JsonObject } from "@fcast/bridge-types";

interface NativePort {
  postMessage(message: JsonObject): void;
  disconnect(): void;
  onMessage: { addListener(listener: (message: unknown) => void): void };
  onDisconnect?: { addListener(listener: () => void): void };
}

interface RuntimeLike {
  sendNativeMessage(host: string, message: JsonObject): Promise<BridgeResponse>;
  connectNative?: (host: string) => NativePort;
}

declare const browser: {
  runtime: RuntimeLike;
  tabs?: {
    executeScript(tabId: number, details: { file: string }): Promise<unknown>;
  };
};
const nativeHost = "com.caniko.fcast_web_sender";
const injectedTabs = new Set<number>();
const state: { current: ExtensionState } = { current: initialState };
let nativePort: NativePort | undefined;
const pending = new Map<string, { resolve: (response: BridgeResponse) => void; reject: (error: Error) => void; timer: number }>();
const eventListeners = new Set<(event: BridgeEvent) => void>();

function receiveNativeMessage(message: unknown): void {
  if (typeof message !== "object" || message === null) return;
  if ("event" in message) {
    const event = message as BridgeEvent;
    applyBridgeEvent(event);
    for (const listener of eventListeners) listener(event);
    return;
  }
  const id = (message as { id?: unknown }).id;
  if (typeof id !== "string") return;
  const request = pending.get(id);
  if (!request) return;
  pending.delete(id);
  globalThis.clearTimeout(request.timer);
  request.resolve(message as BridgeResponse);
}

function getNativePort(): NativePort | undefined {
  if (nativePort) return nativePort;
  nativePort = browser.runtime.connectNative?.(nativeHost);
  if (!nativePort) return undefined;
  nativePort.onMessage.addListener(receiveNativeMessage);
  nativePort.onDisconnect?.addListener(() => {
    nativePort = undefined;
    for (const request of pending.values()) {
      globalThis.clearTimeout(request.timer);
      request.reject(new Error("native companion disconnected"));
    }
    pending.clear();
  });
  return nativePort;
}

export async function bridgeCall(method: BridgeMethod, params: JsonObject): Promise<BridgeResponse> {
  const id = crypto.randomUUID();
  const port = getNativePort();
  if (!port) {
    const response = await browser.runtime.sendNativeMessage(nativeHost, { v: 1, id, method, params });
    if (!response.ok && response.error) state.current = reduce(state.current, { type: "session/error", message: response.error.message });
    return response;
  }
  return new Promise((resolve, reject) => {
    const timer = globalThis.setTimeout(() => {
      pending.delete(id);
      reject(new Error(`bridge request timed out: ${method}`));
    }, 10_000);
    pending.set(id, { resolve, reject, timer });
    port.postMessage({ v: 1, id, method, params });
  });
}

export function currentState(): ExtensionState {
  return state.current;
}

export async function injectDetector(tabId: number): Promise<void> {
  if (injectedTabs.has(tabId)) return;
  if (!browser.tabs) throw new Error("Firefox tabs API is unavailable");
  await browser.tabs.executeScript(tabId, { file: "content/content.js" });
  injectedTabs.add(tabId);
}

export function invalidateDetector(tabId: number): void {
  injectedTabs.delete(tabId);
}

export function ingestCandidates(candidates: unknown[]): void {
  for (const candidate of candidates) {
    if (typeof candidate === "object" && candidate !== null && "id" in candidate) {
      state.current = reduce(state.current, { type: "candidate/upsert", candidate: candidate as never });
    }
  }
}

export function applyBridgeEvent(event: BridgeEvent): void {
  state.current = reduceBridgeEvent(state.current, event);
}

export function subscribeBridgeEvents(listener: (event: BridgeEvent) => void): () => void {
  eventListeners.add(listener);
  return () => eventListeners.delete(listener);
}

export function connectNativeEvents(): () => void {
  const port = getNativePort();
  return () => {
    if (port === nativePort) {
      port?.disconnect();
      nativePort = undefined;
    }
  };
}
