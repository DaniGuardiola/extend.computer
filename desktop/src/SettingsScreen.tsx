import { AccountPanel, type AccountFlow } from "./Account";
import { useState } from "react";
import { DeviceInformation } from "./DeviceInformation";
import { DeviceNameSettings } from "./DeviceNameSettings";
import { Appearance, type Theme } from "./Appearance";
import { ArrowLeft } from "lucide-react";
import {
  PermissionDetails,
  type PermissionDetailsProps,
} from "./PermissionsDialog";

export function SettingsScreen({
  account,
  onBack,
  theme,
  onThemeChange,
  ...permissions
}: PermissionDetailsProps & {
  account: AccountFlow;
  onBack: () => void;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
}) {
  const [advanced, setAdvanced] = useState(false);
  return (
    <>
      <button className="button quiet -ms-3.5 mb-4" onClick={onBack}>
        <ArrowLeft size={15} aria-hidden="true" /> Back
      </button>
      <h1 className="mb-7 text-base font-medium">Settings</h1>
      <section aria-labelledby="account-heading" className="mb-8">
        <h2 id="account-heading" className="mb-4 text-sm font-medium">
          Account
        </h2>
        <AccountPanel account={account} />
      </section>
      <DeviceNameSettings />
      <section
        aria-labelledby="appearance-heading"
        className="mb-8 flex items-center justify-between gap-4"
      >
        <h2 id="appearance-heading" className="text-sm font-medium">
          Appearance
        </h2>
        <Appearance theme={theme} onChange={onThemeChange} />
      </section>
      <section aria-labelledby="permissions-heading">
        <h2 id="permissions-heading" className="text-sm font-medium">
          Permissions
        </h2>
        <div className="mt-5">
          <PermissionDetails {...permissions} />
        </div>
      </section>
      <details
        className="mt-6 border-t border-line pt-5 text-xs text-muted"
        onToggle={(event) => setAdvanced(event.currentTarget.open)}
      >
        <summary className="cursor-pointer">Advanced</summary>
        {advanced && <DeviceInformation snapshot={permissions.snapshot} />}
      </details>
    </>
  );
}
