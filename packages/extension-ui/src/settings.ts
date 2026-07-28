export interface SettingsActions {
  setContinuousDetection?: (enabled: boolean) => void;
}

export function renderSettings(root: HTMLElement, continuousDetection = false, actions: SettingsActions = {}): void {
  root.replaceChildren();
  const heading = document.createElement("h1");
  heading.textContent = "FCast Sender settings";
  root.append(heading);
  const section = document.createElement("section");
  section.setAttribute("aria-labelledby", "detection-heading");
  const sectionHeading = document.createElement("h2");
  sectionHeading.id = "detection-heading";
  sectionHeading.textContent = "Detection";
  section.append(sectionHeading);
  const label = document.createElement("label");
  label.textContent = "Allow continuous media detection on approved sites";
  const checkbox = document.createElement("input");
  checkbox.type = "checkbox";
  checkbox.checked = continuousDetection;
  checkbox.addEventListener("change", () => actions.setContinuousDetection?.(checkbox.checked));
  label.prepend(checkbox);
  section.append(label);
  root.append(section);
}
