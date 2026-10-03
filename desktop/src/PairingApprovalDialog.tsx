import { VerificationSymbols } from "./VerificationSymbols";
import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { Dialog } from "./Dialog";
import { api, type Request } from "./bridge";
export function PairingApprovalDialog({
  request,
  onDone,
}: {
  request: Request;
  onDone: () => Promise<void>;
}) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const answer = async (allow: boolean) => {
    setBusy(true);
    try {
      await api.answer(request.id, allow ? "remember" : "deny");
      await onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      title={
        request.kind === "control"
          ? "Allow this device to control your computer?"
          : request.kind === "verify"
            ? "Do these match?"
            : "Pair with this device?"
      }
      onClose={() => void answer(false)}
    >
      {request.kind === "control" && (
        <p className="text-sm leading-6">
          This device is signed in to your account. Allow its mouse and keyboard
          while you stay signed in.
        </p>
      )}
      {request.kind === "pair" && (
        <p className="text-sm leading-6">
          Another device entered your pairing code. Pair only if you’re
          expecting it.
        </p>
      )}
      {request.kind === "verify" && request.symbols && (
        <VerificationSymbols symbols={request.symbols} />
      )}
      <details className="group mt-7 border-t border-line pt-3 text-xs text-muted">
        <summary className="flex cursor-pointer list-none items-center justify-between py-2 [&::-webkit-details-marker]:hidden">
          Device identity
          <ChevronDown
            size={14}
            aria-hidden="true"
            className="shrink-0 transition-transform group-open:rotate-180"
          />
        </summary>
        <p className="mt-2 select-text break-all font-mono leading-5">
          {request.peer}
        </p>
      </details>
      {error && (
        <p className="error mt-4" role="alert">
          {error}
        </p>
      )}
      <div className="mt-6 flex justify-end gap-2">
        <button
          className="button quiet"
          disabled={busy}
          onClick={() => void answer(false)}
        >
          {request.kind === "verify" ? "They don’t match" : "Decline"}
        </button>
        <button
          className="button primary"
          disabled={busy}
          onClick={() => void answer(true)}
        >
          {request.kind === "control"
            ? "Allow control"
            : request.kind === "verify"
              ? "They match"
              : "Pair device"}
        </button>
      </div>
    </Dialog>
  );
}
