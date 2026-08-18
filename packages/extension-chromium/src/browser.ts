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

interface ScriptingLike {
  executeScript(details: {
    target: { tabId: number };
    files: string[];
    world?: "ISOLATED" | "MAIN";
  }): Promise<unknown>;
}

interface CookieLike {
  name: string;
  value: string;
}

declare const chrome: {
  runtime: RuntimeLike & { sendMessage(message: JsonObject): Promise<unknown> };
  scripting?: ScriptingLike;
  permissions: { request(permissions: { origins: string[] }): Promise<boolean> };
  cookies: {
    getAll(details: { url: string; storeId?: string }): Promise<CookieLike[]>;
    getAllCookieStores(): Promise<Array<{ id: string; tabIds: number[] }>>;
  };
};

const state: { current: ExtensionState } = { current: initialState };
const nativeHost = "com.caniko.fcast_web_sender";
const injectedTabs = new Set<number>();
let nativePort: NativePort | undefined;
const pending = new Map<string, { resolve: (response: BridgeResponse) => void; reject: (error: Error) => void; timer: number }>();
const eventListeners = new Set<(event: BridgeEvent) => void>();
const observedAuthorization = new Map<string, { value: string; observedAt: number }>();
const credentialTtlSeconds = 60;

export function observeAuthorization(tabId: number, url: string, value: string): void {
  const parsed = new URL(url);
  if (parsed.protocol !== "https:") return;
  const key = `${tabId}:${parsed.origin}`;
  const observed = { value, observedAt: Date.now() };
  observedAuthorization.set(key, observed);
  globalThis.setTimeout(() => {
    if (observedAuthorization.get(key) === observed) observedAuthorization.delete(key);
  }, credentialTtlSeconds * 1000);
}

export function clearObservedCredentials(tabId: number): void {
  for (const key of observedAuthorization.keys()) if (key.startsWith(`${tabId}:`)) observedAuthorization.delete(key);
}

export function getObservedAuthorization(tabId: number, origin: string): string | undefined {
  const key = `${tabId}:${origin}`;
  const observed = observedAuthorization.get(key);
  if (!observed || Date.now() - observed.observedAt > credentialTtlSeconds * 1000) {
    observedAuthorization.delete(key);
    return undefined;
  }
  return observed.value;
}

export async function acquireCredentialLease(tabId: number, receiverId: string, url: string): Promise<string | undefined> {
  const parsed = new URL(url);
  if (parsed.protocol !== "https:") throw new Error("Credentials may only be sent for HTTPS media");
  const hello = await bridgeCall("bridge.hello", {});
  const capabilities = hello.ok && typeof hello.result === "object" && hello.result !== null && !Array.isArray(hello.result)
    ? hello.result.capabilities
    : undefined;
  if (!Array.isArray(capabilities) || !capabilities.includes("credentialLeases")) throw new Error("Update the native companion to use credentialed media");
  if (!await chrome.permissions.request({ origins: [`${parsed.origin}/*`] })) throw new Error(`Permission denied for ${parsed.origin}`);

  const storeId = (await chrome.cookies.getAllCookieStores()).find((store) => store.tabIds.includes(tabId))?.id;
  const cookies = await chrome.cookies.getAll({ url, ...(storeId ? { storeId } : {}) });
  const authorization = await chrome.runtime.sendMessage({ source: "fcast-web-sender", type: "credential-authorization", tabId, origin: parsed.origin });
  const headers: JsonObject = {};
  if (cookies.length > 0) headers.Cookie = cookies.map(({ name, value }) => `${name}=${value}`).join("; ");
  if (typeof authorization === "string") headers.Authorization = authorization;
  if (Object.keys(headers).length === 0) return undefined;

  const response = await bridgeCall("credentialLease.create", { receiverId, url, headers, ttlSeconds: credentialTtlSeconds });
  const leaseId = response.ok && typeof response.result === "object" && response.result !== null && !Array.isArray(response.result)
    ? response.result.credentialLeaseId
    : undefined;
  if (typeof leaseId !== "string") throw new Error(response.error?.message ?? "Companion did not return a credential lease ID");
  return leaseId;
}

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

function setCompanionState(next: ExtensionState["companion"]["state"]): void {
  state.current = reduce(state.current, { type: "companion/status", status: { state: next } });
}

function getNativePort(): NativePort | undefined {
  if (nativePort) return nativePort;
  nativePort = chrome.runtime.connectNative?.(nativeHost);
  if (!nativePort) {
    setCompanionState("missing");
    return undefined;
  }
  nativePort.onMessage.addListener(receiveNativeMessage);
  nativePort.onDisconnect?.addListener(() => {
    nativePort = undefined;
    setCompanionState("missing");
    for (const request of pending.values()) {
      globalThis.clearTimeout(request.timer);
      request.reject(new Error("native companion disconnected"));
    }
    pending.clear();
  });
  setCompanionState("ready");
  return nativePort;
}

export async function bridgeCall(method: BridgeMethod, params: JsonObject): Promise<BridgeResponse> {
  const id = crypto.randomUUID();
  const port = getNativePort();
  if (!port) {
    const response = await chrome.runtime.sendNativeMessage(nativeHost, { v: 1, id, method, params });
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

export async function startDiscovery(): Promise<void> {
  await bridgeCall("bridge.hello", {});
  await bridgeCall("discovery.start", {});
  const response = await bridgeCall("receiver.list", {});
  if (response.ok && Array.isArray(response.result)) {
    for (const receiver of response.result) {
      if (typeof receiver === "object" && receiver !== null) state.current = reduce(state.current, { type: "receiver/upsert", receiver: receiver as never });
    }
  }
}

export async function injectDetector(tabId: number): Promise<void> {
  if (injectedTabs.has(tabId)) return;
  if (!chrome.scripting) throw new Error("Chromium scripting API is unavailable");
  await chrome.scripting.executeScript({ target: { tabId }, files: ["content/main-world-probe.js"], world: "MAIN" });
  await chrome.scripting.executeScript({ target: { tabId }, files: ["content/content.js"] });
  injectedTabs.add(tabId);
}

export function invalidateDetector(tabId: number): void {
  injectedTabs.delete(tabId);
}

export function currentState(): ExtensionState {
  return state.current;
}

export function ingestCandidates(candidates: unknown[], tabId: number): void {
  for (const candidate of candidates) {
    if (typeof candidate === "object" && candidate !== null && "id" in candidate) {
      state.current = reduce(state.current, { type: "candidate/upsert", candidate: { ...candidate, tabId } as never });
    }
  }
}

export function removeCandidatesForTab(tabId: number): void {
  for (const candidate of Object.values(state.current.candidates)) {
    if (candidate.tabId === tabId) state.current = reduce(state.current, { type: "candidate/remove", candidateId: candidate.id });
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
