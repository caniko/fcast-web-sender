import { renderSettings } from "@fcast/extension-ui";

const root = document.querySelector<HTMLElement>("#settings");
if (root) {
  const storage = (globalThis as unknown as { chrome?: { storage?: { local?: { get(key: string): Promise<Record<string, unknown>>; set(values: Record<string, unknown>): Promise<void> } } } }).chrome?.storage?.local;
  void storage?.get("continuousDetection").then((values) => renderSettings(root, values.continuousDetection === true, {
    setContinuousDetection: (enabled) => void storage?.set({ continuousDetection: enabled }),
  }));
  if (!storage) renderSettings(root);
}
