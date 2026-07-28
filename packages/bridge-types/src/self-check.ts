import { BRIDGE_METHODS, BRIDGE_PROTOCOL_VERSION } from "./generated.ts";

if (BRIDGE_PROTOCOL_VERSION !== 1 || !BRIDGE_METHODS.includes("bridge.hello")) {
  throw new Error("bridge schema generation is inconsistent");
}
