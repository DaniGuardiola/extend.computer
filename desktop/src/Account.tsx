import { useEffect, useState, type FormEvent } from "react";
import {
  Fingerprint,
  LogOut,
  Monitor,
  RefreshCw,
  Trash2,
  ExternalLink,
} from "lucide-react";
import {
  accounts,
  native,
  type AccountState,
  type AccountDevice,
  type Peer,
} from "./bridge";
import { Dialog } from "./Dialog";
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
        if (!account.busy) onClose();
      }}
    >
      <AccountPanel account={account} />
    </Dialog>
  );
}
export function AccountPanel({ account }: { account: AccountFlow }) {
  const { state, busy, error, act } = account;
  const [server, setServer] = useState(
    state?.server ?? "https://extend.computer",
  );
  useEffect(() => {
    if (state?.server) setServer(state.server);
  }, [state?.server]);
  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const email = String(data.get("email"));
    const password = String(data.get("password"));
    await act(async () => {
      if (server !== state?.server) await accounts.configure(server);
      return accounts.login(email, password);
    });
  }
  async function browser() {
    await act(async () => {
      if (server !== state?.server) await accounts.configure(server);
      return accounts.browser();
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
          <p className="text-xs text-muted break-words">{state.server}</p>
          <p className="text-muted">
            {state.device_id
              ? "This computer is registered to your account. Account presence updates while the app is open."
              : "Signed in. Device registration will retry when the server is reachable."}
          </p>
          <div className="flex flex-wrap gap-2">
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
            Finish signing in in your browser. Passkeys, security keys, and
            two-factor authentication work there.
          </p>
          <button
            className="button quiet"
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
              ? "authenticator app or a recovery code"
              : "recovery code"}
            .
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
                ? "Authenticator or recovery code"
                : "Recovery code"}
              <input
                name="code"
                autoComplete="one-time-code"
                maxLength={39}
                required
                autoFocus
                spellCheck={false}
              />
            </label>
            <button className="button" disabled={busy}>
              Verify code
            </button>
          </form>
          {state.pending.keys && (
            <button
              className="button quiet"
              disabled={busy}
              onClick={() => void browser()}
            >
              <Fingerprint size={15} /> Use passkey or security key in browser
            </button>
          )}
          <button
            className="button quiet"
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
                required
                maxLength={254}
              />
            </label>
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
            <button className="button" disabled={busy || !state}>
              {busy ? "Signing in…" : "Log in"}
            </button>
          </form>
          <button
            className="button quiet"
            disabled={busy || !state}
            onClick={() => void browser()}
          >
            <Fingerprint size={15} /> Sign in with browser
          </button>
          <div className="flex flex-wrap gap-3">
            <button
              className="text-muted underline underline-offset-4"
              disabled={busy}
              onClick={() => void act(() => accounts.open("signup"))}
            >
              Create account on website{" "}
              <ExternalLink className="inline" size={12} />
            </button>
            <button
              className="text-muted underline underline-offset-4"
              disabled={busy}
              onClick={() => void act(() => accounts.open("recover"))}
            >
              Forgot password?
            </button>
          </div>
          <details className="text-xs text-muted">
            <summary className="cursor-pointer">Account server</summary>
            <label className="field mt-3 grid gap-2">
              Server URL
              <input
                type="url"
                value={server}
                onChange={(e) => setServer(e.target.value)}
                disabled={busy}
              />
            </label>
            <button
              className="button quiet mt-2"
              disabled={busy}
              onClick={() => void act(() => accounts.configure(server))}
            >
              Use server
            </button>
            <p className="mt-2">
              Use your own HTTPS server, or localhost for development.
            </p>
          </details>
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
          className="button quiet"
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
              <Monitor size={19} />
              <div className="min-w-0 flex-1">
                <p className="truncate">{device.name}</p>
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
                className="icon-button"
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
          These devices need local pairing. Updated apps connect through your account.
        </p>
      )}
    </section>
  );
}
