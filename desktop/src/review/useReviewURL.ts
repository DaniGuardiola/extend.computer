import { useMemo, useSyncExternalStore } from "react";
import {
  readReviewURL,
  writeReviewURL,
  type ReviewURLState,
} from "./url-state";

const changed = "review-url-change";
function subscribe(callback: () => void) {
  window.addEventListener(changed, callback);
  window.addEventListener("popstate", callback);
  return () => {
    window.removeEventListener(changed, callback);
    window.removeEventListener("popstate", callback);
  };
}
const snapshot = () => location.href;

// The URL is the durable source of truth, including across a full HMR reload.
export function useReviewURL() {
  const href = useSyncExternalStore(subscribe, snapshot);
  const state = useMemo(() => readReviewURL(href), [href]);
  const update = (patch: Partial<ReviewURLState>) => {
    const next = { ...readReviewURL(location.href), ...patch };
    const url = writeReviewURL(location.href, next);
    if (url.href === location.href) return;
    history.replaceState(history.state, "", url);
    window.dispatchEvent(new Event(changed));
  };
  return [state, update] as const;
}
