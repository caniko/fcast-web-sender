const SOURCE = "fcast-web-sender";
let channel: string | undefined;

window.addEventListener("message", (event: MessageEvent) => {
  if (event.source !== window || event.data?.source !== SOURCE) return;
  if (event.data.type === "probe-init" && typeof event.data.channel === "string") {
    channel = event.data.channel;
    return;
  }
  if (!channel || event.data.channel !== channel || event.data.type !== "media-command") return;
  const id = String(event.data.elementId ?? "");
  const target = document.querySelector<HTMLMediaElement>(`[data-fcast-element-id="${CSS.escape(id)}"]`);
  if (!target) return;
  if (event.data.command === "play") void target.play();
  if (event.data.command === "pause") target.pause();
  if (event.data.command === "stop") {
    target.pause();
    target.currentTime = 0;
  }
  if (event.data.command === "seek" && Number.isFinite(event.data.positionSeconds)) target.currentTime = event.data.positionSeconds;
  if (event.data.command === "volume" && Number.isFinite(event.data.level)) target.volume = Math.max(0, Math.min(1, event.data.level));
});
