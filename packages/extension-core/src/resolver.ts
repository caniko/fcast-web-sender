import type { DeliveryPath, MediaCandidate, ReceiverCapabilities } from "./model.js";

export interface CapabilitySet extends ReceiverCapabilities {}

export interface ResolverInput {
  media?: MediaCandidate;
  capabilities: CapabilitySet;
  preferredPath?: DeliveryPath;
  captureAuthorized?: boolean;
}

export interface DeliveryDecision {
  path?: DeliveryPath;
  reason: string;
  attempted: DeliveryPath[];
}

function pathAllowed(path: DeliveryPath, input: ResolverInput): boolean {
  const { media, capabilities } = input;
  if (!media || media.encrypted) return false;
  switch (path) {
    case "direct":
      return capabilities.direct && media.kind === "direct-url" && Boolean(media.url) && !media.requiresCredentials;
    case "fcompanion":
      return capabilities.fcompanion && Boolean(media.url) && (media.requiresCredentials || media.kind === "hls" || media.kind === "dash");
    case "element-capture":
      return capabilities.elementCapture && input.captureAuthorized !== false;
    case "tab-mirroring":
      return capabilities.tabMirroring && input.captureAuthorized === true;
  }
}

export function resolveDeliveryPath(input: ResolverInput): DeliveryDecision {
  const order: DeliveryPath[] = input.preferredPath
    ? [input.preferredPath, "direct", "fcompanion", "element-capture", "tab-mirroring"]
    : ["direct", "fcompanion", "element-capture", "tab-mirroring"];
  const attempted: DeliveryPath[] = [];
  for (const path of order) {
    if (attempted.includes(path)) continue;
    attempted.push(path);
    if (pathAllowed(path, input)) {
      return { path, attempted, reason: `${path} is available for this candidate and receiver` };
    }
  }
  const reason = input.media?.encrypted
    ? "encrypted media cannot be transferred without DRM support"
    : "no compatible delivery path is available";
  return { attempted, reason };
}

export function chooseDeliveryPath(media: MediaCandidate | undefined, capabilities: CapabilitySet): DeliveryPath | undefined {
  return resolveDeliveryPath({ media, capabilities }).path;
}
