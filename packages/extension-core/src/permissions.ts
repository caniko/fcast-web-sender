export const ACTIVE_TAB_PERMISSION = "activeTab" as const;

export function permissionExplanation(): string {
  return "The sender requests access to the current tab only after you choose a receiver or capture action.";
}
