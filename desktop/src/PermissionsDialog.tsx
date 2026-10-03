import type { ReactNode } from "react";
import { Check, ExternalLink } from "lucide-react";
import { Dialog } from "./Dialog";
import { type Snapshot } from "./bridge";
export type PermissionKey = "listen" | "post" | "wifi";
export type PermissionAction = "receive" | "share";
export type PermissionDetailsProps = {
  snapshot: Snapshot;
  checked: boolean;
  error: string;
  openPermission: (permission: PermissionKey) => Promise<void>;
};

function PermissionRow({
  title,
  description,
  checked,
  allowed,
  issue = "Permission needed",
  children,
}: {
  title: string;
  description: string;
  checked: boolean;
  allowed: boolean;
  issue?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 border-t border-line py-4">
      <div className="min-w-0">
        <h3 className="text-sm font-medium">{title}</h3>
        <p className="mt-1 text-xs text-muted">{description}</p>
        {checked && !allowed && (
          <p className="mt-2 flex items-center gap-1.5 text-xs text-attention">
            <span
              className="size-1.5 shrink-0 rounded-full bg-current"
              aria-hidden="true"
            />
            {issue}
          </p>
        )}
      </div>
      {checked ? (
        children
      ) : (
        <span className="shrink-0 text-sm text-muted">Checking…</span>
      )}
    </div>
  );
}

export function PermissionDetails({
  snapshot,
  checked,
  error,
  openPermission,
}: PermissionDetailsProps) {
  const allowed = snapshot.permissions.post;
  return (
    <>
      {checked && !snapshot.permissions.available && (
        <p className="error mb-4">
          The input helper is missing. Reinstall extend.computer.
        </p>
      )}
      {error && (
        <p className="error mb-4" role="alert">
          {error}
        </p>
      )}
      <PermissionRow
        title="Accessibility"
        description="Read and control keyboard and mouse."
        checked={checked}
        allowed={allowed}
      >
        <button
          className={`button quiet shrink-0 whitespace-nowrap ${allowed ? "text-success" : ""}`}
          title="Open Accessibility"
          aria-label={
            allowed ? "Allowed — Open Accessibility" : "Open Accessibility"
          }
          onClick={() => void openPermission("post")}
          disabled={!snapshot.permissions.available}
        >
          {allowed && <Check size={16} aria-hidden="true" />}
          {allowed ? "Allowed" : "Open settings"}
          <ExternalLink size={14} aria-hidden="true" />
        </button>
      </PermissionRow>
      <PermissionRow
        title="Wi-Fi optimization"
        description="Keep your keyboard and mouse responsive over Wi-Fi. AirDrop pauses while connected."
        checked={checked}
        allowed={snapshot.permissions.wifi}
        issue={
          snapshot.permissions.wifi_installing
            ? "Setting up…"
            : snapshot.permissions.wifi_setup_failed
              ? "Wi-Fi optimization could not start"
              : "Permission needed"
        }
      >
        <button
          className={`button quiet shrink-0 whitespace-nowrap ${snapshot.permissions.wifi ? "text-success" : ""}`}
          title="Open Wi-Fi optimization settings"
          aria-label={
            snapshot.permissions.wifi
              ? "Allowed — Open Wi-Fi optimization settings"
              : snapshot.permissions.wifi_setup_failed
                ? "Retry Wi-Fi optimization setup"
                : "Open Wi-Fi optimization settings"
          }
          disabled={snapshot.permissions.wifi_installing}
          onClick={() => void openPermission("wifi")}
        >
          {snapshot.permissions.wifi && <Check size={16} aria-hidden="true" />}
          {snapshot.permissions.wifi
            ? "Allowed"
            : snapshot.permissions.wifi_installing
              ? "Setting up…"
              : snapshot.permissions.wifi_setup_failed
                ? "Try again"
                : "Open settings"}
          {!snapshot.permissions.wifi_installing && (
            <ExternalLink size={14} aria-hidden="true" />
          )}
        </button>
      </PermissionRow>
      {snapshot.permissions.wifi_setup_failed && (
        <p className="error text-xs" role="alert">
          Wi-Fi optimization is unavailable. Try again to repair its setup.
        </p>
      )}
    </>
  );
}

export function PermissionsDialog({
  action,
  onClose,
  ...details
}: PermissionDetailsProps & {
  action: PermissionAction;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={
        action === "receive"
          ? "Allow remote control"
          : "Allow keyboard & mouse sharing"
      }
      onClose={onClose}
    >
      <p className="mb-5 text-sm leading-6 text-muted">
        {action === "receive"
          ? "Allow access below so your other device can control this Mac."
          : "Allow access below to share your keyboard and mouse between devices."}
      </p>
      <PermissionDetails {...details} />
      {action === "share" &&
        details.checked &&
        details.snapshot.permissions.post &&
        !details.snapshot.permissions.listen && (
          <p className="mt-2 text-xs text-muted">
            Accessibility is enabled. Restart extend.computer to refresh input
            access.
          </p>
        )}
    </Dialog>
  );
}
