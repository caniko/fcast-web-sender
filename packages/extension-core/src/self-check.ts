import { candidateFromProbe } from "./detector.ts";
import { chooseDeliveryPath } from "./resolver.ts";
import { initialState, reduce } from "./state.ts";

const candidate = candidateFromProbe({ id: "video-1", mediaKind: "video", sourceUrl: "https://media.example/video.m3u8", playing: true, visibleArea: 100 });
const path = chooseDeliveryPath(candidate, { direct: true, fcompanion: true, elementCapture: true, tabMirroring: true });
if (path !== "fcompanion") throw new Error(`unexpected fallback path: ${path}`);

const state = reduce(initialState, {
  type: "receiver/select",
  receiverId: "receiver-1",
});
if (state.selectedReceiverId !== "receiver-1") throw new Error("state reducer failed");

const stale = reduce(state, { type: "receiver/upsert", revision: -1, receiver: {
  id: "receiver-1",
  name: "stale",
  host: "192.0.2.1",
  port: 5353,
  trusted: false,
  connectionState: "discovered",
} });
if (stale.receivers["receiver-1"] !== undefined) throw new Error("stale revision was accepted");
