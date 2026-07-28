export interface FirefoxMirrorAdapter {
  captureCurrentTab(): Promise<MediaStream>;
}

export function firefoxMirrorAdapter(): FirefoxMirrorAdapter {
  return {
    async captureCurrentTab() {
      const devices = globalThis.navigator?.mediaDevices;
      if (!devices?.getDisplayMedia) throw new Error("Firefox display-media capture is unavailable");
      // Firefox owns the source picker and requires this call to originate from
      // a visible user gesture. The extension deliberately does not bypass it.
      return devices.getDisplayMedia({ video: true, audio: true });
    },
  };
}
