import type { BridgeEvent } from "@fcast/bridge-types";
import type { ExtensionState, MediaCandidate, Receiver, Session } from "./model.js";

export type Action =
  | { type: "receiver/upsert"; receiver: Receiver; revision?: number }
  | { type: "receiver/remove"; receiverId: string; revision?: number }
  | { type: "receiver/connection"; receiverId: string; state: Receiver["connectionState"]; revision?: number }
  | { type: "receiver/select"; receiverId?: string }
  | { type: "candidate/upsert"; candidate: MediaCandidate }
  | { type: "candidate/remove"; candidateId: string }
  | { type: "candidate/select"; candidateId?: string }
  | { type: "session/upsert"; session: Session; revision?: number }
  | { type: "session/remove"; sessionId: string; revision?: number }
  | { type: "session/error"; message: string }
  | { type: "companion/status"; status: ExtensionState["companion"] }
  | { type: "clear-error" };

export const initialState: ExtensionState = {
  revision: 0,
  receivers: {},
  candidates: {},
  sessions: {},
  companion: { state: "missing" },
};

function isStale(state: ExtensionState, revision: number | undefined): boolean {
  return revision !== undefined && revision < state.revision;
}

export function reduce(state: ExtensionState, action: Action): ExtensionState {
  const revision = "revision" in action ? action.revision : undefined;
  if (isStale(state, revision)) return state;
  const nextRevision = Math.max(state.revision, revision ?? state.revision);
  switch (action.type) {
    case "receiver/upsert":
      return { ...state, revision: nextRevision, receivers: { ...state.receivers, [action.receiver.id]: action.receiver } };
    case "receiver/remove": {
      const receivers = { ...state.receivers };
      delete receivers[action.receiverId];
      return {
        ...state,
        revision: nextRevision,
        receivers,
        selectedReceiverId: state.selectedReceiverId === action.receiverId ? undefined : state.selectedReceiverId,
      };
    }
    case "receiver/connection": {
      const receiver = state.receivers[action.receiverId];
      if (!receiver) return state;
      return { ...state, revision: nextRevision, receivers: { ...state.receivers, [receiver.id]: { ...receiver, connectionState: action.state } } };
    }
    case "receiver/select":
      return { ...state, selectedReceiverId: action.receiverId };
    case "candidate/upsert":
      return { ...state, candidates: { ...state.candidates, [action.candidate.id]: action.candidate } };
    case "candidate/remove": {
      const candidates = { ...state.candidates };
      delete candidates[action.candidateId];
      return { ...state, candidates, selectedCandidateId: state.selectedCandidateId === action.candidateId ? undefined : state.selectedCandidateId };
    }
    case "candidate/select":
      return { ...state, selectedCandidateId: action.candidateId };
    case "session/upsert":
      return { ...state, revision: nextRevision, sessions: { ...state.sessions, [action.session.id]: action.session }, lastError: undefined };
    case "session/remove": {
      const sessions = { ...state.sessions };
      delete sessions[action.sessionId];
      return { ...state, revision: nextRevision, sessions };
    }
    case "session/error":
      return { ...state, lastError: action.message };
    case "companion/status":
      return { ...state, companion: action.status };
    case "clear-error":
      return { ...state, lastError: undefined };
  }
}

export function reduceBridgeEvent(state: ExtensionState, event: BridgeEvent): ExtensionState {
  const data = event.data;
  if (typeof data !== "object" || data === null || Array.isArray(data)) return state;
  const record = data as Record<string, unknown>;
  const revision = typeof record.revision === "number" ? record.revision : undefined;
  switch (event.event) {
    case "receiver.upsert":
      return typeof record.id === "string" ? reduce(state, { type: "receiver/upsert", receiver: record as unknown as Receiver, revision }) : state;
    case "receiver.remove":
      return typeof record.id === "string" ? reduce(state, { type: "receiver/remove", receiverId: record.id, revision }) : state;
    case "receiver.connectionState":
      return typeof record.id === "string" && typeof record.state === "string"
        ? reduce(state, { type: "receiver/connection", receiverId: record.id, state: record.state as Receiver["connectionState"], revision })
        : state;
    case "session.state":
      return updateSessionByReceiver(state, record, revision, (session) => ({
        ...session,
        state: typeof record.state === "string" ? (record.state.toLowerCase() as Session["state"]) : session.state,
      }));
    case "session.progress":
      return updateSessionByReceiver(state, record, revision, (session) => ({
        ...session,
        positionMs: typeof record.positionMs === "number" ? record.positionMs : session.positionMs,
        durationMs: typeof record.durationMs === "number" ? record.durationMs : session.durationMs,
      }));
    case "session.volume":
      return updateSessionByReceiver(state, record, revision, (session) => ({
        ...session,
        volume: typeof record.volume === "number" ? record.volume : session.volume,
      }));
    case "session.speed":
      return updateSessionByReceiver(state, record, revision, (session) => ({
        ...session,
        speed: typeof record.speed === "number" ? record.speed : session.speed,
      }));
    case "session.tracks":
      return updateSessionByReceiver(state, record, revision, (session) => ({
        ...session,
        tracks: Array.isArray(record.tracks) ? (record.tracks as Session["tracks"]) : session.tracks,
      }));
    case "session.trackSelected":
      return state;
    case "bridge.warning":
      return typeof record.message === "string" ? reduce(state, { type: "session/error", message: record.message }) : state;
    default:
      return state;
  }
}

function updateSessionByReceiver(
  state: ExtensionState,
  record: Record<string, unknown>,
  revision: number | undefined,
  update: (session: Session) => Session,
): ExtensionState {
  if (typeof record.receiverId !== "string") return state;
  const session = Object.values(state.sessions).find((entry) => entry.receiverId === record.receiverId);
  if (!session) return state;
  return reduce(state, { type: "session/upsert", session: update(session), revision });
}
