import type { Device, Permissions } from "./bridge";

export type PendingPermissionAction =
  | { kind: "receive" }
  | { kind: "share"; peer: string; device: Device }
  | { kind: "display"; peer: string; mode: "extend" | "mirror" };

export function permissionsReady(
  kind: PendingPermissionAction["kind"],
  access: Permissions,
  route: "local" | "usb" = "local",
) {
  // Sharing this screen needs only Screen Recording.
  if (kind === "display") return access.screen;
  return (
    access.available &&
    access.post &&
    // Only Wi-Fi connections need Wi-Fi optimization.
    (access.wifi || access.wired || route === "usb") &&
    (kind === "receive" || access.listen)
  );
}

// A permission grant may arrive through several polls/focus events. Consume the
// user's intent before executing it so those events cannot start duplicate work.
export class PermissionContinuation {
  private action: PendingPermissionAction | null = null;

  waitFor(action: PendingPermissionAction) {
    this.action = action;
  }

  cancel() {
    this.action = null;
  }

  takeReady(
    access: Permissions,
    currentRoute: (action: PendingPermissionAction) => "local" | "usb" = () =>
      "local",
  ): PendingPermissionAction | null {
    if (
      !this.action ||
      !permissionsReady(this.action.kind, access, currentRoute(this.action))
    )
      return null;
    const action = this.action;
    this.action = null;
    return action;
  }
}
