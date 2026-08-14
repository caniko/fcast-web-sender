import { bridgeCall, clearObservedCredentials, connectNativeEvents, currentState, getObservedAuthorization, ingestCandidates, injectDetector, invalidateDetector, observeAuthorization, removeCandidatesForTab, subscribeBridgeEvents } from "./browser.js";
import { firefoxMirrorAdapter } from "./capture.js";
import { createMirrorPeer } from "./mirroring.js";

declare const browser: {
  runtime: {
    onStartup: { addListener(listener: () => void): void };
    onMessage: { addListener(listener: (message: unknown, sender: { tab?: { id?: number } }, sendResponse: (response?: unknown) => void) => void): void };
  };
  tabs: {
    query(queryInfo: { active: boolean; lastFocusedWindow: boolean }): Promise<Array<{ id?: number }>>;
    onActivated: { addListener(listener: (activeInfo: { tabId: number }) => void): void };
    onUpdated: { addListener(listener: (tabId: number, changeInfo: { status?: string }) => void): void };
  };
  webRequest: {
    onBeforeSendHeaders: { addListener(listener: (details: { tabId: number; url: string; requestHeaders?: Array<{ name: string; value?: string }> }) => void, filter: { urls: string[]; types: string[] }, extraInfoSpec: string[]): void };
  };
  browserAction: { onClicked: { addListener(listener: (tab: { id?: number }) => void): void } };
};
let activeTabId: number | undefined;
void browser.tabs.query({ active: true, lastFocusedWindow: true }).then(([tab]) => { activeTabId = tab?.id; });
browser.tabs.onActivated.addListener(({ tabId }) => { activeTabId = tabId; });
browser.webRequest.onBeforeSendHeaders.addListener((details) => {
  if (details.tabId !== activeTabId) return;
  const authorization = details.requestHeaders?.find(({ name }) => name.toLowerCase() === "authorization")?.value;
  if (authorization) observeAuthorization(details.tabId, details.url, authorization);
}, { urls: ["http://*/*", "https://*/*"], types: ["media", "xmlhttprequest"] }, ["requestHeaders"]);
browser.runtime.onStartup.addListener(() => {
  connectNativeEvents();
  void bridgeCall("discovery.start", {});
});
browser.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (typeof message !== "object" || message === null) return;
  const credentialRequest = message as { source?: unknown; type?: unknown; tabId?: unknown; origin?: unknown };
  if (!sender.tab && credentialRequest.source === "fcast-web-sender" && credentialRequest.type === "credential-authorization" && typeof credentialRequest.tabId === "number" && typeof credentialRequest.origin === "string") {
    sendResponse(getObservedAuthorization(credentialRequest.tabId, credentialRequest.origin));
    return;
  }
  const candidates = (message as { candidates?: unknown }).candidates;
  if ((message as { source?: unknown }).source === "fcast-web-sender" && sender.tab?.id !== undefined && Array.isArray(candidates)) {
    ingestCandidates(candidates, sender.tab.id);
  }
});
browser.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status === "loading") {
    invalidateDetector(tabId);
    clearObservedCredentials(tabId);
    removeCandidatesForTab(tabId);
  }
});
let activeMirror: ReturnType<typeof createMirrorPeer> | undefined;
let removeMirrorEvents: (() => void) | undefined;
browser.browserAction.onClicked.addListener((tab) => {
  if (tab.id === undefined) return;
  void (async () => {
    await injectDetector(tab.id!);
    await new Promise((resolve) => globalThis.setTimeout(resolve, 100));
    const state = currentState();
    const receiver = Object.values(state.receivers).find((entry) => entry.trusted);
    if (!receiver) return;
    const stream = await firefoxMirrorAdapter().captureCurrentTab();
    activeMirror?.close();
    removeMirrorEvents?.();
    activeMirror = createMirrorPeer(stream);
    const sessionId = `mirror-${crypto.randomUUID()}`;
    removeMirrorEvents = subscribeBridgeEvents((event) => {
      if (event.event !== "mirror.answer" || typeof event.data !== "object" || event.data === null || Array.isArray(event.data)) return;
      const data = event.data as { sessionId?: unknown; sdp?: unknown };
      if (data.sessionId === sessionId && typeof data.sdp === "string") void activeMirror?.applyAnswer(data.sdp);
    });
    await bridgeCall("receiver.connect", { receiverId: receiver.id });
    await bridgeCall("mirror.negotiate", { phase: "start", sessionId, receiverId: receiver.id });
    await bridgeCall("mirror.negotiate", { phase: "offer", sessionId, sdp: await activeMirror.offer() });
  })().catch((error: unknown) => console.error("FCast mirror failed", error));
});
