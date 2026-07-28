import { bridgeCall, connectNativeEvents, currentState, ingestCandidates, injectDetector, invalidateDetector, subscribeBridgeEvents } from "./browser.js";
import { firefoxMirrorAdapter } from "./capture.js";
import { createMirrorPeer } from "./mirroring.js";

declare const browser: {
  runtime: {
    onStartup: { addListener(listener: () => void): void };
    onMessage: { addListener(listener: (message: unknown) => void): void };
  };
  tabs: { onUpdated: { addListener(listener: (tabId: number, changeInfo: { status?: string }) => void): void } };
  browserAction: { onClicked: { addListener(listener: (tab: { id?: number }) => void): void } };
};
browser.runtime.onStartup.addListener(() => {
  connectNativeEvents();
  void bridgeCall("discovery.start", {});
});
browser.runtime.onMessage.addListener((message) => {
  if (typeof message !== "object" || message === null) return;
  const candidates = (message as { candidates?: unknown }).candidates;
  if ((message as { source?: unknown }).source === "fcast-web-sender" && Array.isArray(candidates)) ingestCandidates(candidates);
});
browser.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status === "loading") invalidateDetector(tabId);
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
