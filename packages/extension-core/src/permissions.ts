export const ACTIVE_TAB_PERMISSION = "activeTab" as const;

export function permissionExplanation(): string {
  return "A local companion is required. After you choose media, the extension may request access to that site so it can send a short-lived Cookie or Authorization lease to your trusted receiver. It also watches the active tab for Authorization headers on HTTPS media. Nothing is uploaded to a cloud service.";
}
