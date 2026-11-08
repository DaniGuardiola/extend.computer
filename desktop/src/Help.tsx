import { useEffect, useState } from "react";
import { Check, Copy, ExternalLink } from "lucide-react";
import { support } from "./bridge";

/** Report a problem, and the diagnostic info a report can include. */
export function Help() {
  // Kept current while Settings is open, so what is shown is what is copied.
  const [info, setInfo] = useState<string | null>(null);
  const [infoError, setInfoError] = useState("");
  const [shown, setShown] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const next = await support.diagnosticInfo();
        if (active) {
          setInfo(next);
          setInfoError("");
        }
      } catch (e) {
        if (active) setInfoError(String(e));
      }
      if (active) timer = setTimeout(load, 2000);
    };
    void load();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, []);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);
  async function copy() {
    if (info === null) return;
    try {
      await navigator.clipboard.writeText(info);
      setCopyFailed(false);
      setCopied(true);
    } catch {
      // Show the text so it can be selected and copied by hand.
      setCopyFailed(true);
      setShown(true);
    }
  }
  async function report() {
    setError("");
    try {
      await support.reportProblem();
    } catch (e) {
      setError(String(e));
    }
  }
  return (
    <section aria-labelledby="help-heading" className="my-8">
      <h2 id="help-heading" className="mb-4 text-sm font-medium">
        Help
      </h2>
      <div className="flex flex-wrap items-center gap-3">
        <button className="button secondary" onClick={() => void report()}>
          Report a problem
          <ExternalLink size={12} aria-hidden="true" />
        </button>
        <button
          className="button quiet"
          disabled={info === null}
          onClick={() => void copy()}
        >
          {copied ? (
            <Check size={14} aria-hidden="true" />
          ) : (
            <Copy size={14} aria-hidden="true" />
          )}
          {copied ? "Copied" : "Copy diagnostic info"}
        </button>
      </div>
      <p className="mt-3 text-xs text-muted">
        Report a problem opens a new GitHub issue with the app and system
        versions filled in. Diagnostic info adds connection and permission
        states, without device names, keys, addresses or your email.
      </p>
      <span
        className={copyFailed ? "mt-3 block text-xs text-attention" : "sr-only"}
        role="status"
      >
        {copyFailed
          ? "Couldn’t copy. Select the text to copy it."
          : copied
            ? "Diagnostic info copied"
            : ""}
      </span>
      {(error || infoError) && (
        <p role="alert" className="error mt-3">
          {error || infoError}
        </p>
      )}
      {info !== null && (
        <details
          className="mt-3 text-xs text-muted"
          open={shown}
          onToggle={(event) => setShown(event.currentTarget.open)}
        >
          <summary className="cursor-pointer">Show diagnostic info</summary>
          <pre className="mt-3 select-text overflow-x-auto rounded-lg border border-line bg-surface p-3 font-mono text-xs leading-5 whitespace-pre-wrap break-words text-ink">
            {info}
          </pre>
        </details>
      )}
    </section>
  );
}
