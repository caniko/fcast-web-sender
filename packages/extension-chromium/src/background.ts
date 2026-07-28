import { bridgeCall, connectNativeEvents, currentState, ingestCandidates, injectDetector, invalidateDetector, startDiscovery, subscribeBridgeEvents } from "./browser.js";
import { chromiumCaptureAdapter } from "./capture.js";
import { createMirrorPeer } from "./mirroring.js";

declare const chrome: {
  runtime: {
    onInstalled: { addListener(listener: () => void): void };
    onMessage: { addListener(listener: (message: unknown) => void): void };
  };
  tabs: { onUpdated: { addListener(listener: (tabId: number, changeInfo: { status?: string }) => void): void } };
  action: { onClicked: { addListener(listener: (tab: { id?: number }) => void): void } };
};

chrome.runtime.onInstalled.addListener(() => {
  connectNativeEvents();
  void startDiscovery();
});
let activeMirror: ReturnType<typeof createMirrorPeer> | undefined;
let removeMirrorEvents: (() => void) | undefined;
chrome.runtime.onMessage.addListener((message) => {
  if (typeof message !== "object" || message === null) return;
  const candidates = (message as { source?: unknown; type?: unknown; candidates?: unknown }).candidates;
  if ((message as { source?: unknown }).source === "fcast-web-sender" && Array.isArray(candidates)) ingestCandidates(candidates);
});
chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status === "loading") invalidateDetector(tabId);
});
chrome.action.onClicked.addListener((tab) => {
  if (tab.id === undefined) return;
  void (async () => {
    await startDiscovery();
    await injectDetector(tab.id!);
    await new Promise((resolve) => globalThis.setTimeout(resolve, 100));
    const state = currentState();
    const receiver = Object.values(state.receivers).find((entry) => entry.trusted);
    const candidate = Object.values(state.candidates).sort((left, right) => right.confidence - left.confidence)[0];
    if (!receiver || !candidate) return;
    const stream = await chromiumCaptureAdapter().captureTab(tab.id!);
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
    const sdp = await activeMirror.offer();
    await bridgeCall("mirror.negotiate", { phase: "offer", sessionId, sdp });
  })().catch((error: unknown) => console.error("FCast mirror failed", error));
});
