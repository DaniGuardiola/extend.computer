import { useEffect, useState, type FormEvent } from "react";
import {
  Fingerprint,
  LogOut,
  RefreshCw,
  Trash2,
  ExternalLink,
  Globe,
} from "lucide-react";
import {
  accounts,
  native,
  type AccountState,
  type AccountDevice,
  type Peer,
} from "./bridge";
import { Dialog } from "./Dialog";

const OFFICIAL_SERVER = "https://extend.computer";
/** Matches the backend's origin check: a bare host means HTTPS. */
function normalizeServer(value: string) {
  const raw = value.trim().replace(/\/+$/, "");
  return raw.includes("://") ? raw : `https://${raw}`;
}
/** HTTPS servers are shown as their host; others keep their scheme. */
function displayServer(server: string) {
  return server.replace(/^https:\/\//, "");
}
import { DeviceIcon } from "./DeviceIcon";
import { TruncatedText } from "./TruncatedText";
export function useAccount() {
  const [state, setState] = useState<AccountState | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!native) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const next = await accounts.status();
        if (active) setState(next);
      } catch (e) {
        if (active) setError(String(e));
      }
      if (active) timer = setTimeout(poll, 1000);
    }
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, []);
  async function act(work: () => Promise<AccountState | void>) {
    setBusy(true);
    setError("");
    try {
      const next = await work();
      if (next) setState(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return { state, busy, error, act };
}
export type AccountFlow = ReturnType<typeof useAccount>;
export function AccountDialog({
  account,
  onClose,
}: {
  account: AccountFlow;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={account.state?.email ? "Your account" : "Log in"}
      onClose={() => {
        if (account.busy) return;
        // Closing abandons an unfinished sign-in instead of leaving it waiting.
        if (account.state?.browser_pending || account.state?.pending)
          void account.act(() => accounts.cancel());
        onClose();
      }}
    >
      <AccountPanel account={account} />
    </Dialog>
  );
}
export function AccountPanel({ account }: { account: AccountFlow }) {
  const { state, busy, error, act } = account;
  const official = !state?.server || state.server === OFFICIAL_SERVER;
  const [custom, setCustom] = useState(!official);
  const [server, setServer] = useState(
    official ? "" : displayServer(state.server),
  );
  useEffect(() => {
    if (!state?.server) return;
    const isOfficial = state.server === OFFICIAL_SERVER;
    setCustom(!isOfficial);
    setServer(isOfficial ? "" : displayServer(state.server));
  }, [state?.server]);
  // The backend accepts a bare host (HTTPS assumed) or a full origin.
  const chosen = custom && server.trim() ? server.trim() : OFFICIAL_SERVER;
  async function useChosenServer() {
    if (normalizeServer(chosen) !== state?.server)
      await accounts.configure(chosen);
  }
  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const email = String(data.get("email"));
    const password = String(data.get("password"));
    await act(async () => {
      await useChosenServer();
      return accounts.login(email, password);
    });
  }
  async function browser() {
    await act(async () => {
      await useChosenServer();
      return accounts.browser();
    });
  }
  const [passkeys, setPasskeys] = useState(false);
  useEffect(() => {
    void accounts
      .passkeyAvailable()
      .then(setPasskeys)
      .catch(() => setPasskeys(false));
  }, []);
  async function passkey() {
    await act(async () => {
      await useChosenServer();
      return accounts.passkey();
    });
  }
  async function openPage(page: "signup" | "recover") {
    await act(async () => {
      await useChosenServer();
      return accounts.open(page);
    });
  }
  const problem = error || state?.error;
  return (
    <div className="space-y-4 text-sm">
      {problem && (
        <p role="alert" className="error break-words">
          {problem}
        </p>
      )}
      {!native ? (
        <p className="text-muted">
          Account sign-in is available in the desktop app.
        </p>
      ) : state?.email ? (
        <>
          <p className="break-words">{state.email}</p>
          {state.server !== OFFICIAL_SERVER && (
            <p className="text-xs text-muted break-words">
              Using a custom account server: {displayServer(state.server)}
            </p>
          )}
          <p className="text-muted">
            {state.device_id
              ? "This device is registered to your account."
              : "Signed in. Device registration will retry when the server is reachable."}
          </p>
          <div className="-mx-3.5 flex flex-wrap gap-2">
            <button
              className="button quiet"
              disabled={busy}
              onClick={() => void act(() => accounts.open("account"))}
            >
              <ExternalLink size={14} /> Manage account &amp; security
            </button>
            <button
              className="button quiet"
              disabled={busy}
              onClick={() => void act(() => accounts.logout())}
            >
              <LogOut size={14} /> Sign out
            </button>
          </div>
        </>
      ) : state?.browser_pending ? (
        <>
          <p>
            Finish signing in in your browser. You'll be signed in here as soon
            as you're done.
          </p>
          <button
            className="button quiet -ms-3.5"
            disabled={busy}
            onClick={() => void act(() => accounts.cancel())}
          >
            Cancel browser sign-in
          </button>
        </>
      ) : state?.pending ? (
        <>
          <p>
            Confirm with your{" "}
            {state.pending.totp
              ? "one-time code or a recovery code"
              : "recovery code"}
            {state.pending.keys && ", or a passkey or security key"}.
          </p>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              const code = String(
                new FormData(event.currentTarget).get("code"),
              );
              void act(() => accounts.verify(code));
            }}
            className="space-y-4"
          >
            <label className="field grid gap-2">
              {state.pending.totp
                ? "One-time or recovery code"
                : "Recovery code"}
              <input
                name="code"
                autoComplete="one-time-code"
                inputMode="numeric"
                maxLength={9}
                required
                autoFocus
                spellCheck={false}
              />
            </label>
            <button className="button primary w-full" disabled={busy}>
              Verify code
            </button>
          </form>
          {state.pending.keys &&
            (passkeys && state.server === OFFICIAL_SERVER ? (
              <button
                className="button quiet -ms-3.5"
                disabled={busy}
                onClick={() => void act(() => accounts.passkey(true))}
              >
                <Fingerprint size={15} /> Use a passkey or security key
              </button>
            ) : (
              <button
                className="button quiet -ms-3.5"
                disabled={busy}
                onClick={() => void browser()}
              >
                <Fingerprint size={15} /> Use passkey or security key in browser
              </button>
            ))}
          <button
            className="button quiet -ms-3.5"
            disabled={busy}
            onClick={() => void act(() => accounts.cancel())}
          >
            Start over
          </button>
        </>
      ) : (
        <>
          <form onSubmit={login} className="space-y-4">
            <label className="field grid gap-2">
              Email
              <input
                name="email"
                type="email"
                autoComplete="username"
                placeholder="you@example.com"
                required
                maxLength={254}
              />
            </label>
            <div className="grid gap-2">
              <label className="field grid gap-2">
                Password
                <input
                  name="password"
                  type="password"
                  autoComplete="current-password"
                  minLength={12}
                  maxLength={1024}
                  required
                />
              </label>
              <button
                type="button"
                className="link-button justify-self-end"
                disabled={busy}
                onClick={() => void openPage("recover")}
              >
                Forgot your password?
                <ExternalLink size={12} aria-hidden="true" />
              </button>
            </div>
            <button className="button primary w-full" disabled={busy || !state}>
              {busy ? "Signing in…" : "Sign in"}
            </button>
          </form>
          <div className="flex items-center gap-3 text-xs text-muted">
            <span className="h-px flex-1 bg-line" />
            or
            <span className="h-px flex-1 bg-line" />
          </div>
          <div className="grid gap-2">
            {passkeys && !custom && (
              <button
                className="button secondary w-full"
                disabled={busy || !state}
                onClick={() => void passkey()}
              >
                <Fingerprint size={15} aria-hidden="true" /> Sign in with a
                passkey
              </button>
            )}
            <button
              className="button secondary w-full"
              disabled={busy || !state}
              onClick={() => void browser()}
            >
              <Globe size={15} aria-hidden="true" /> Sign in with your browser
            </button>
          </div>
          <p className="text-center text-xs text-muted">
            Need an account?{" "}
            <button
              type="button"
              className="link-button"
              disabled={busy}
              onClick={() => void openPage("signup")}
            >
              Create an account
              <ExternalLink size={12} aria-hidden="true" />
            </button>
          </p>
          <div className="space-y-3 pt-6 text-xs">
            <div className="flex items-center justify-between gap-3">
              <label className="flex cursor-pointer items-center gap-2">
                <input
                  type="checkbox"
                  checked={custom}
                  disabled={busy}
                  onChange={(event) => setCustom(event.target.checked)}
                />
                Use a custom account server
              </label>
              <button
                type="button"
                className="link-button"
                onClick={() => void act(() => accounts.open("account-servers"))}
              >
                Learn more
                <ExternalLink size={12} aria-hidden="true" />
              </button>
            </div>
            {custom && (
              <label className="field">
                <input
                  aria-label="Server address"
                  value={server}
                  onChange={(event) => setServer(event.target.value)}
                  placeholder="accounts.example.com"
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  disabled={busy}
                />
              </label>
            )}
          </div>
        </>
      )}
    </div>
  );
}
export function AccountDevices({
  account,
  peers,
  onConnect,
  onPair,
  disabled = false,
}: {
  account: AccountFlow;
  peers: Peer[];
  onConnect: (peer: Peer) => void;
  onPair: () => void;
  disabled?: boolean;
}) {
  const { state, busy, error, act } = account;
  const [removal, setRemoval] = useState<AccountDevice | null>(null);
  if (!state?.email) return null;
  const devices = state.devices.filter(
    (device) =>
      device.id !== state.device_id &&
      !peers.some((peer) => peer.id === device.fingerprint),
  );
  if (!devices.length) return null;
  return (
    <section
      aria-labelledby="account-devices-heading"
      className="mt-7 border-t border-line pt-5"
    >
      <div className="mb-4 flex items-center justify-between">
        <h2 id="account-devices-heading" className="text-sm font-medium">
          Account devices
        </h2>
        <button
          className="button quiet -me-3.5"
          aria-label="Refresh account devices"
          disabled={busy}
          onClick={() => void act(() => accounts.refresh())}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      {(error || state.error) && (
        <p role="alert" className="error mb-3">
          {error || state.error}
        </p>
      )}
      {!devices.length && (
        <p className="text-xs text-muted">
          Log in on another computer to see it here.
        </p>
      )}
      <div className="space-y-3">
        {devices.map((device) => {
          const paired = peers.find((peer) => peer.id === device.fingerprint);
          return (
            <div
              key={device.id}
              className="flex items-center gap-3 rounded-xl border border-line p-3"
            >
              <DeviceIcon platform={device.platform} size={19} />
              <div className="min-w-0 flex-1">
                <TruncatedText>{device.name}</TruncatedText>
                <p className="text-xs text-muted">
                  {device.platform} ·{" "}
                  {state.error
                    ? "Presence unavailable"
                    : device.online
                      ? "Signed in"
                      : "Offline"}
                  {paired ? " · Paired" : ""}
                </p>
              </div>
              {paired ? (
                <button
                  className="button quiet"
                  disabled={
                    busy || disabled || paired.availability !== "online"
                  }
                  onClick={() => onConnect(paired)}
                >
                  Connect
                </button>
              ) : (
                <button
                  className="button quiet"
                  disabled={busy || disabled || !device.online}
                  onClick={onPair}
                >
                  Pair
                </button>
              )}
              <button
                className="icon-button -me-2.25"
                aria-label={"Remove " + device.name + " from account"}
                disabled={busy}
                onClick={() => setRemoval(device)}
              >
                <Trash2 size={14} />
              </button>
            </div>
          );
        })}
      </div>
      {removal && (
        <Dialog
          title="Remove account device?"
          onClose={() => !busy && setRemoval(null)}
        >
          <p className="text-sm">
            Remove {removal.name} from this account? Its account presence will
            stop. Local pairing stays available.
          </p>
          {error && (
            <p role="alert" className="error mt-3">
              {error}
            </p>
          )}
          <div className="mt-5 flex justify-end gap-2">
            <button
              className="button quiet"
              disabled={busy}
              onClick={() => setRemoval(null)}
            >
              Cancel
            </button>
            <button
              className="button destructive"
              disabled={busy}
              onClick={() =>
                void act(async () => {
                  const next = await accounts.remove(removal.id);
                  setRemoval(null);
                  return next;
                })
              }
            >
              Remove device
            </button>
          </div>
        </Dialog>
      )}
      {!!devices.length && (
        <p className="mt-3 text-xs text-muted">
          These devices need local pairing. Updated apps connect through your
          account.
        </p>
      )}
    </section>
  );
}
