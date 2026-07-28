export interface CaptureAdapter {
  captureTab(tabId: number): Promise<MediaStream>;
}

export function chromiumCaptureAdapter(): CaptureAdapter {
  return {
    async captureTab(tabId) {
      const browser = globalThis as unknown as { chrome?: { tabCapture?: { capture(options: { audio: boolean; video: boolean }, callback: (stream?: MediaStream) => void): void } } };
      const capture = browser.chrome?.tabCapture?.capture;
      if (!capture) throw new Error("Chromium tab capture is unavailable");
      return new Promise((resolve, reject) => capture.call(browser.chrome?.tabCapture, { audio: true, video: true }, (stream) => stream ? resolve(stream) : reject(new Error(`tab capture failed for ${tabId}`))));
    },
  };
}
