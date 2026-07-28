import type { BridgeMethod, BridgeResponse, JsonObject, JsonValue } from "@fcast/bridge-types";

export type ReceiverConnectionState =
  | "discovered"
  | "trustRequired"
  | "connecting"
  | "connected"
  | "disconnected"
  | "mismatch";

export type SessionState =
  | "idle"
  | "resolving"
  | "loading"
  | "buffering"
  | "playing"
  | "paused"
  | "seeking"
  | "stopping"
  | "stopped"
  | "failed";

export type CandidateKind =
  | "direct-url"
  | "hls"
  | "dash"
  | "blob"
  | "media-source"
  | "media-stream"
  | "capture-only";

export interface MediaCandidate {
  id: string;
  tabId?: number;
  frameId?: number;
  elementId?: string;
  kind: CandidateKind;
  mediaKind: "audio" | "video";
  url?: string;
  mimeType?: string;
  title?: string;
  poster?: string;
  width?: number;
  height?: number;
  durationMs?: number;
  positionMs: number;
  playing: boolean;
  muted: boolean;
  visibleArea: number;
  lastInteractionAt?: number;
  encrypted: boolean;
  live: boolean;
  requiresCredentials: boolean;
  confidence: number;
  rankingExplanation: string[];
  observedAt: number;
}

export interface ReceiverCapabilities {
  direct: boolean;
  fcompanion: boolean;
  elementCapture: boolean;
  tabMirroring: boolean;
  containers?: string[];
  codecs?: string[];
  subtitles?: string[];
}

export interface Receiver {
  id: string;
  name: string;
  serviceName?: string;
  host: string;
  addresses?: string[];
  port: number;
  protocolVersion?: number;
  fingerprint?: string;
  trusted: boolean;
  connectionState: ReceiverConnectionState;
  capabilities?: ReceiverCapabilities;
  ttlSeconds?: number;
}

export type DeliveryPath = "direct" | "fcompanion" | "element-capture" | "tab-mirroring";

export interface Session {
  id: string;
  receiverId: string;
  candidateId?: string;
  state: SessionState;
  mode?: DeliveryPath;
  revision: number;
  mediaUrl?: string;
  positionMs: number;
  durationMs?: number;
  volume: number;
  speed: number;
  tracks: Track[];
  recoverable?: boolean;
  lastError?: string;
}

export interface Track {
  id: string;
  kind: "audio" | "video" | "text";
  label?: string;
  language?: string;
  selected: boolean;
}

export interface CompanionStatus {
  state: "missing" | "starting" | "negotiating" | "ready" | "disconnected" | "incompatible";
  protocolVersion?: number;
  version?: string;
  capabilities?: string[];
}

export interface ExtensionState {
  revision: number;
  receivers: Record<string, Receiver>;
  candidates: Record<string, MediaCandidate>;
  sessions: Record<string, Session>;
  selectedReceiverId?: string;
  selectedCandidateId?: string;
  companion: CompanionStatus;
  lastError?: string;
}

export type BridgeCall = <T extends JsonValue = JsonValue>(
  method: BridgeMethod,
  params: JsonObject,
) => Promise<BridgeResponse<T>>;
