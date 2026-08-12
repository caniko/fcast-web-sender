import { renderPopup } from "@fcast/extension-ui";
import { resolveDeliveryPath } from "@fcast/extension-core";
import { acquireCredentialLease, bridgeCall, currentState, injectDetector } from "./browser.js";

declare const browser: { tabs: { query(queryInfo: { active: boolean; currentWindow: boolean }): Promise<Array<{ id?: number }>> } };

const root = document.querySelector<HTMLElement>("#popup");
if (root) {
  const refresh = async () => {
    const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
    if (tab?.id !== undefined) await injectDetector(tab.id);
    await bridgeCall("discovery.start", {});
    await bridgeCall("receiver.list", {});
    const state = currentState();
    const candidates = Object.values(state.candidates).filter((candidate) => candidate.tabId === tab?.id);
    renderPopup(root, Object.values(state.receivers), candidates, Object.values(state.sessions), {
      trustReceiver: (receiver, fingerprint) => void bridgeCall("receiver.trust", { receiverId: receiver.id, fingerprint }),
      castCandidate: async (candidate, receiver) => {
        if (!candidate.url) return;
        const credentialLeaseId = candidate.requiresCredentials
          ? await acquireCredentialLease(candidate.tabId ?? tab?.id ?? -1, receiver.id, candidate.url)
          : undefined;
        if (credentialLeaseId && (candidate.kind === "hls" || candidate.kind === "dash")) {
          await bridgeCall("credentialLease.revoke", { credentialLeaseId });
          throw new Error("Authenticated adaptive streams require dynamic FCompanion resources, which this companion SDK does not support");
        }
        const decision = resolveDeliveryPath({
          media: { ...candidate, requiresCredentials: Boolean(credentialLeaseId) },
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
          ...(credentialLeaseId ? { credentialLeaseId } : {}),
        });
        await refresh();
      },
      controlSession: async (session, action) => {
        await bridgeCall("session.control", { sessionId: session.id, action });
        await refresh();
      },
    });
  };
  void refresh();
}
