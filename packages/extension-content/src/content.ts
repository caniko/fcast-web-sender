import { candidateFromProbe, rankCandidates, type MediaProbe } from "@fcast/extension-core";

const SOURCE = "fcast-web-sender";
const channel = crypto.randomUUID();
const elementIds = new WeakMap<HTMLMediaElement, string>();
const encrypted = new WeakSet<HTMLMediaElement>();
const lastInteraction = new WeakMap<HTMLMediaElement, number>();
const listeners = new WeakMap<HTMLMediaElement, Array<() => void>>();
let pending = false;

function elementId(element: HTMLMediaElement): string {
  const existing = elementIds.get(element);
  if (existing) return existing;
  const id = `media-${crypto.randomUUID()}`;
  elementIds.set(element, id);
  element.dataset.fcastElementId = id;
  return id;
}

function visibleArea(element: HTMLElement): number {
  const rect = element.getBoundingClientRect();
  const width = Math.max(0, Math.min(rect.right, window.innerWidth) - Math.max(rect.left, 0));
  const height = Math.max(0, Math.min(rect.bottom, window.innerHeight) - Math.max(rect.top, 0));
  return width * height;
}

function sourceUrl(element: HTMLMediaElement): string | undefined {
  return element.currentSrc || element.src || element.querySelector("source")?.src || undefined;
}

function needsCredentials(element: HTMLMediaElement, url: string | undefined): boolean {
  if (element.crossOrigin === "use-credentials") return true;
  if (!url) return false;
  try {
    return new URL(url, location.href).origin === location.origin;
  } catch {
    return false;
  }
}

function probe(element: HTMLMediaElement): MediaProbe {
  const url = sourceUrl(element);
  return {
    id: elementId(element),
    mediaKind: element.tagName.toLowerCase() === "audio" ? "audio" : "video",
    sourceUrl: url,
    mimeType: element.querySelector("source")?.type || undefined,
    title: document.title || undefined,
    poster: element instanceof HTMLVideoElement ? element.poster || undefined : undefined,
    width: element instanceof HTMLVideoElement ? element.videoWidth || undefined : undefined,
    height: element instanceof HTMLVideoElement ? element.videoHeight || undefined : undefined,
    durationMs: Number.isFinite(element.duration) ? element.duration * 1000 : undefined,
    positionMs: Number.isFinite(element.currentTime) ? element.currentTime * 1000 : 0,
    playing: !element.paused && !element.ended,
    muted: element.muted || element.volume === 0,
    visibleArea: visibleArea(element),
    lastInteractionAt: lastInteraction.get(element),
    encrypted: encrypted.has(element),
    live: !Number.isFinite(element.duration),
    requiresCredentials: needsCredentials(element, url),
    observedAt: Date.now(),
  };
}

function emit(): void {
  pending = false;
  const candidates = rankCandidates(
    Array.from(document.querySelectorAll<HTMLMediaElement>("audio,video"), (element) => candidateFromProbe(probe(element))),
  );
  const message = { source: SOURCE, channel, type: "media-detected", candidates };
  window.postMessage(message, window.location.origin);
  const runtime = (globalThis as unknown as {
    chrome?: { runtime?: { sendMessage?: (message: unknown) => void } };
    browser?: { runtime?: { sendMessage?: (message: unknown) => void } };
  });
  runtime.chrome?.runtime?.sendMessage?.(message);
  runtime.browser?.runtime?.sendMessage?.(message);
}

function scheduleEmit(): void {
  if (pending) return;
  pending = true;
  window.setTimeout(emit, 50);
}

function attach(element: HTMLMediaElement): void {
  if (listeners.has(element)) return;
  elementId(element);
  const onChange = () => scheduleEmit();
  const onInteraction = () => {
    lastInteraction.set(element, Date.now());
    scheduleEmit();
  };
  const onEncrypted = () => {
    encrypted.add(element);
    scheduleEmit();
  };
  const events = ["play", "pause", "loadedmetadata", "durationchange", "emptied", "volumechange", "ratechange", "ended"];
  for (const event of events) element.addEventListener(event, onChange);
  for (const event of ["pointerdown", "keydown"]) element.addEventListener(event, onInteraction);
  element.addEventListener("encrypted", onEncrypted);
  listeners.set(element, [
    ...events.map((event) => () => element.removeEventListener(event, onChange)),
    ...["pointerdown", "keydown"].map((event) => () => element.removeEventListener(event, onInteraction)),
    () => element.removeEventListener("encrypted", onEncrypted),
  ]);
}

function detach(element: HTMLMediaElement): void {
  for (const remove of listeners.get(element) ?? []) remove();
  listeners.delete(element);
}

const observer = new MutationObserver((mutations) => {
  for (const mutation of mutations) {
    if (mutation.type === "childList") {
      for (const node of Array.from(mutation.addedNodes)) {
        if (node instanceof HTMLMediaElement) attach(node);
        if (node instanceof Element) node.querySelectorAll<HTMLMediaElement>("audio,video").forEach(attach);
      }
      for (const node of Array.from(mutation.removedNodes)) {
        if (node instanceof HTMLMediaElement) detach(node);
        if (node instanceof Element) node.querySelectorAll<HTMLMediaElement>("audio,video").forEach(detach);
      }
    }
    if (mutation.type === "attributes" && mutation.target instanceof HTMLMediaElement) attach(mutation.target);
  }
  scheduleEmit();
});

for (const element of Array.from(document.querySelectorAll<HTMLMediaElement>("audio,video"))) attach(element);
observer.observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: ["src", "poster"] });
window.postMessage({ source: SOURCE, channel, type: "probe-init" }, window.location.origin);
emit();

function stop(): void {
  observer.disconnect();
  for (const element of Array.from(document.querySelectorAll<HTMLMediaElement>("audio,video"))) detach(element);
}

window.addEventListener("pagehide", stop, { once: true });
