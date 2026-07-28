import type { CandidateKind, MediaCandidate } from "./model.js";

export interface MediaProbe {
  id: string;
  mediaKind: "audio" | "video";
  sourceUrl?: string;
  mimeType?: string;
  title?: string;
  poster?: string;
  width?: number;
  height?: number;
  durationMs?: number;
  positionMs?: number;
  playing?: boolean;
  muted?: boolean;
  visibleArea?: number;
  lastInteractionAt?: number;
  encrypted?: boolean;
  live?: boolean;
  requiresCredentials?: boolean;
  observedAt?: number;
}

export function classifySource(sourceUrl: string | undefined, mimeType = ""): CandidateKind {
  if (!sourceUrl) return "capture-only";
  if (sourceUrl.startsWith("blob:")) return "blob";
  if (sourceUrl.startsWith("mediasource:")) return "media-source";
  const lowerUrl = sourceUrl.toLowerCase();
  const lowerMimeType = mimeType.toLowerCase();
  if (lowerMimeType.includes("mpegurl") || /\.m3u8(?:$|[?#])/.test(lowerUrl)) return "hls";
  if (lowerMimeType.includes("dash+xml") || /\.mpd(?:$|[?#])/.test(lowerUrl)) return "dash";
  if (/^https?:/i.test(sourceUrl)) return "direct-url";
  return "capture-only";
}

export function candidateFromProbe(probe: MediaProbe): MediaCandidate {
  const kind = classifySource(probe.sourceUrl, probe.mimeType);
  const explanation: string[] = [];
  let confidence = 0;
  if (probe.playing) {
    confidence += 100;
    explanation.push("currently playing");
  }
  if (probe.sourceUrl && /^https?:/i.test(probe.sourceUrl)) {
    confidence += 40;
    explanation.push("has an HTTP(S) source");
  }
  if (kind === "hls" || kind === "dash") {
    confidence += 35;
    explanation.push(`adaptive ${kind.toUpperCase()} manifest`);
  }
  if ((probe.visibleArea ?? 0) > 0) {
    confidence += 50;
    explanation.push("visible media element");
  } else {
    confidence -= 50;
    explanation.push("hidden or zero-area element");
  }
  if ((probe.durationMs ?? 0) > 60_000) {
    confidence += 15;
    explanation.push("long-form media");
  }
  if (probe.mediaKind === "video" && !probe.muted) {
    confidence += 10;
    explanation.push("audible video");
  }
  if (probe.encrypted) {
    confidence -= 100;
    explanation.push("encrypted playback; DRM is unsupported");
  }
  if (kind === "capture-only") explanation.push("requires capture fallback");
  return {
    id: probe.id,
    mediaKind: probe.mediaKind,
    kind,
    url: probe.sourceUrl,
    mimeType: probe.mimeType,
    title: probe.title,
    poster: probe.poster,
    width: probe.width,
    height: probe.height,
    durationMs: probe.durationMs,
    positionMs: probe.positionMs ?? 0,
    playing: probe.playing ?? false,
    muted: probe.muted ?? true,
    visibleArea: probe.visibleArea ?? 0,
    lastInteractionAt: probe.lastInteractionAt,
    encrypted: probe.encrypted ?? false,
    live: probe.live ?? !Number.isFinite(probe.durationMs),
    requiresCredentials: probe.requiresCredentials ?? false,
    confidence,
    rankingExplanation: explanation,
    observedAt: probe.observedAt ?? Date.now(),
  };
}

export function rankCandidates(candidates: readonly MediaCandidate[]): MediaCandidate[] {
  return [...candidates].sort(
    (left, right) => right.confidence - left.confidence || right.observedAt - left.observedAt || left.id.localeCompare(right.id),
  );
}
