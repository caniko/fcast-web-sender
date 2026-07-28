export interface MirrorPeer {
  offer(): Promise<string>;
  applyAnswer(sdp: string): Promise<void>;
  close(): void;
}

export function createMirrorPeer(stream: MediaStream, onState?: (state: RTCPeerConnectionState) => void): MirrorPeer {
  const peer = new RTCPeerConnection({ iceServers: [] });
  for (const track of stream.getTracks()) peer.addTrack(track, stream);
  if (onState) peer.addEventListener("connectionstatechange", () => onState(peer.connectionState));
  return {
    async offer() {
      const offer = await peer.createOffer();
      await peer.setLocalDescription(offer);
      if (peer.iceGatheringState !== "complete") {
        await new Promise<void>((resolve) => {
          const timeout = globalThis.setTimeout(resolve, 5_000);
          peer.addEventListener("icegatheringstatechange", () => {
            if (peer.iceGatheringState === "complete") {
              globalThis.clearTimeout(timeout);
              resolve();
            }
          }, { once: true });
        });
      }
      return peer.localDescription?.sdp ?? "";
    },
    async applyAnswer(sdp) {
      await peer.setRemoteDescription({ type: "answer", sdp });
    },
    close() {
      peer.close();
      for (const track of stream.getTracks()) track.stop();
    },
  };
}
