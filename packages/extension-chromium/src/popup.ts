import { renderPopup } from "@fcast/extension-ui";
import { resolveDeliveryPath } from "@fcast/extension-core";
import { bridgeCall, currentState, injectDetector, startDiscovery } from "./browser.js";

declare const chrome: { tabs: { query(queryInfo: { active: boolean; currentWindow: boolean }): Promise<Array<{ id?: number }>> } };

const root = document.querySelector<HTMLElement>("#popup");
if (root) {
  const refresh = async () => {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (tab?.id !== undefined) await injectDetector(tab.id);
    await startDiscovery();
    const state = currentState();
    renderPopup(root, Object.values(state.receivers), Object.values(state.candidates), Object.values(state.sessions), {
      trustReceiver: (receiver) => {
        if (receiver.fingerprint) void bridgeCall("receiver.trust", { receiverId: receiver.id, fingerprint: receiver.fingerprint });
      },
      castCandidate: async (candidate, receiver) => {
        if (!candidate.url) return;
        const decision = resolveDeliveryPath({
          media: candidate,
          capabilities: { direct: true, fcompanion: true, elementCapture: false, tabMirroring: false },
        });
        if (decision.path !== "direct" && decision.path !== "fcompanion") return;
        await bridgeCall("receiver.connect", { receiverId: receiver.id });
        await bridgeCall("session.load", {
          receiverId: receiver.id,
          url: candidate.url,
          title: candidate.title ?? "",
          contentType: candidate.mimeType ?? "",
          mode: decision.path,
        });
        await refresh();
      },
      controlSession: async (session, action, value) => {
        await bridgeCall("session.control", { sessionId: session.id, action, ...(value === undefined ? {} : { positionMs: value }) });
        await refresh();
      },
    });
  };
  void refresh();
}
