import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type LocalDeviceInfo, type Snapshot } from "./bridge";

function CopyValue({
  value,
  label,
  mono = false,
}: {
  value: string;
  label: string;
  mono?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <dd className="mt-1 flex items-center gap-2 text-ink">
      <span
        className={`min-w-0 select-text break-all ${mono ? "font-mono leading-5" : ""}`}
      >
        {value}
      </span>
      <button
        className="inline-flex size-6 shrink-0 items-center justify-center rounded text-muted hover:bg-line hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2"
        title={copied ? "Copied" : `Copy ${label}`}
        aria-label={copied ? `${label} copied` : `Copy ${label}`}
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(value);
            setError(false);
            setCopied(true);
          } catch {
            setError(true);
          }
        }}
      >
        {copied ? (
          <Check size={14} aria-hidden="true" />
        ) : (
          <Copy size={14} aria-hidden="true" />
        )}
      </button>
      <span className={error ? "text-attention" : "sr-only"} role="status">
        {error
          ? "Couldn’t copy. Select the text to copy it."
          : copied
            ? "Copied"
            : ""}
      </span>
    </dd>
  );
}

export function DeviceInformation({ snapshot }: { snapshot: Snapshot }) {
  const [info, setInfo] = useState<LocalDeviceInfo | null>(null);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    setError("");
    void api.localDeviceInfo().then(
      (value) => {
        if (active) setInfo(value);
      },
      (error) => {
        if (active) setError(String(error));
      },
    );
    return () => {
      active = false;
    };
  }, [attempt]);

  return (
    <div className="mt-5">
      {error ? (
        <div className="mb-4">
          <p role="alert" className="error">
            {error}
          </p>
          <button
            className="button quiet -ms-3.5 mt-2"
            onClick={() => setAttempt(attempt + 1)}
          >
            Try again
          </button>
        </div>
      ) : !info ? (
        <p role="status" className="mb-4">
          Loading device details…
        </p>
      ) : (
        <dl className="space-y-4">
          <div>
            <dt className="select-text font-medium">Device identity</dt>
            <CopyValue value={info.identity} label="device identity" mono />
            <p className="mt-1">
              Public fingerprint used to recognize this device.
            </p>
          </div>
          <div>
            <dt className="select-text font-medium">extend.computer version</dt>
            <CopyValue
              value={`${info.version}${snapshot.development ? " (development)" : ""}`}
              label="version"
            />
          </div>
        </dl>
      )}
      <dl className="mt-4">
        <dt className="select-text font-medium">Local addresses</dt>
        {snapshot.addresses.length ? (
          [...new Set(snapshot.addresses)].map((address) => (
            <CopyValue
              key={address}
              value={address}
              label="local address"
              mono
            />
          ))
        ) : (
          <dd className="mt-1">No local network address available.</dd>
        )}
      </dl>
    </div>
  );
}
