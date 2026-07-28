import { permissionExplanation, type DeliveryPath, type MediaCandidate, type Receiver, type Session } from "@fcast/extension-core";

export interface PopupActions {
  trustReceiver?: (receiver: Receiver) => void;
  castCandidate?: (candidate: MediaCandidate, receiver: Receiver, path?: DeliveryPath) => void;
  controlSession?: (session: Session, action: string, value?: number) => void;
}

export function renderPopup(
  root: HTMLElement,
  receivers: readonly Receiver[],
  candidates: readonly MediaCandidate[] = [],
  sessions: readonly Session[] = [],
  actions: PopupActions = {},
): void {
  root.replaceChildren();
  const heading = document.createElement("h1");
  heading.textContent = "FCast Sender";
  root.append(heading);
  const explanation = document.createElement("p");
  explanation.textContent = permissionExplanation();
  root.append(explanation);

  const status = document.createElement("p");
  status.setAttribute("role", "status");
  status.textContent = receivers.length === 0 ? "Searching for receivers…" : `${receivers.length} receiver(s) available`;
  root.append(status);

  const receiverList = document.createElement("ul");
  receiverList.setAttribute("aria-label", "Receivers");
  for (const receiver of receivers) {
    const item = document.createElement("li");
    const label = receiver.trusted ? receiver.name : `${receiver.name} (needs trust)`;
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.receiverId = receiver.id;
    button.textContent = label;
    button.disabled = receiver.connectionState === "mismatch";
    button.addEventListener("click", () => {
      if (!receiver.trusted) actions.trustReceiver?.(receiver);
      else if (candidates[0]) actions.castCandidate?.(candidates[0], receiver);
    });
    item.append(button);
    receiverList.append(item);
  }
  root.append(receiverList);

  if (candidates.length > 0) {
    const candidatesHeading = document.createElement("h2");
    candidatesHeading.textContent = "Media on this tab";
    root.append(candidatesHeading);
    const candidateList = document.createElement("ul");
    candidateList.setAttribute("aria-label", "Detected media");
    for (const candidate of candidates) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = `${candidate.title || candidate.mediaKind} — ${candidate.kind}`;
      button.title = candidate.rankingExplanation.join("; ");
      button.addEventListener("click", () => {
        const receiver = receivers.find((entry) => entry.trusted);
        if (receiver) actions.castCandidate?.(candidate, receiver);
      });
      item.append(button);
      candidateList.append(item);
    }
    root.append(candidateList);
  }

  for (const session of sessions) {
    const sessionSection = document.createElement("section");
    sessionSection.setAttribute("aria-label", `Session ${session.id}`);
    const sessionStatus = document.createElement("p");
    sessionStatus.textContent = `${session.state} · ${Math.round(session.positionMs / 1000)}s`;
    sessionSection.append(sessionStatus);
    for (const [action, text] of [["play", "Play"], ["pause", "Pause"], ["stop", "Stop"]] as const) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = text;
      button.addEventListener("click", () => actions.controlSession?.(session, action));
      sessionSection.append(button);
    }
    root.append(sessionSection);
  }
}
