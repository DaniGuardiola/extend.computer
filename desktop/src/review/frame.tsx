import { createRoot } from "react-dom/client";
import { createPortal } from "react-dom";
import { useEffect, useState } from "react";
import { App } from "../App";
import { scenarios, type Step } from "./scenarios";
import { isReviewShortcut } from "./shortcuts";
import "../style.css";
import "./frame.css";

if (import.meta.env.MODE !== "ui-review")
  throw new Error("Run npm run ui:review to open the UI review.");
const params = new URLSearchParams(location.search);
const scenario =
  scenarios.find((s) => s.id === params.get("state")) ?? scenarios[0];
window.addEventListener("keydown", (event) => {
  if (!isReviewShortcut(event)) return;
  event.preventDefault();
  parent.postMessage(
    { type: "review-shortcut", id: scenario.id, key: event.key },
    location.origin,
  );
});
const theme =
  params.get("theme") === "system"
    ? "system"
    : params.get("theme") === "dark"
      ? "dark"
      : "light";
localStorage.setItem("extend.computer.appearance", theme);
document.documentElement.dataset.theme =
  theme === "system"
    ? matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light"
    : theme;
// Independent fixtures: per-peer display settings should not leak between cases.
for (const key of Object.keys(localStorage))
  if (key.startsWith("display:")) localStorage.removeItem(key);
function PreviewApp() {
  const [header, setHeader] = useState<HTMLElement | null>(null);
  useEffect(() => setHeader(document.querySelector("header")), []);
  return (
    <>
      <App />
      {header &&
        navigator.platform.startsWith("Mac") &&
        createPortal(
          <div className="review-traffic-lights" aria-hidden="true">
            <i />
            <i />
            <i />
          </div>,
          header,
        )}
    </>
  );
}
createRoot(document.getElementById("root")!).render(<PreviewApp />);
function waitFor<T>(
  read: () => T | undefined,
  description: string,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      observer.disconnect();
      reject(new Error(`Could not open ${description}`));
    }, 8000);
    const check = () => {
      const value = read();
      if (value !== undefined) {
        clearTimeout(timeout);
        observer.disconnect();
        resolve(value);
      }
    };
    const observer = new MutationObserver(check);
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true,
    });
    check();
  });
}
const text = (element: Element) =>
  element.textContent?.replace(/\s+/g, " ").trim();
function visible(element: HTMLElement) {
  return element.getClientRects().length > 0 && !element.closest("[inert]");
}
async function runStep(step: Step) {
  const root = () =>
    document.querySelector<HTMLElement>('[role="dialog"]') ?? document.body;
  if ("field" in step) {
    const input = await waitFor(() => {
      const labelled = [...root().querySelectorAll<HTMLLabelElement>("label")]
        .find((l) => text(l) === step.field)
        ?.querySelector("input");
      const input =
        labelled ??
        [...root().querySelectorAll<HTMLInputElement>("input")].find(
          (i) => i.getAttribute("aria-label") === step.field,
        );
      return input && !input.disabled && visible(input) ? input : undefined;
    }, step.field);
    input.focus();
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, step.value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => resolve()),
    );
    input.blur();
    return;
  }
  const target = await waitFor(() => {
    const candidates = [
      ...("text" in step
        ? document.body
        : root()
      ).querySelectorAll<HTMLElement>(
        'button, summary, label, [role="option"]',
      ),
    ];
    return candidates.find(
      (e) =>
        visible(e) &&
        !e.hasAttribute("disabled") &&
        ("label" in step
          ? e.getAttribute("aria-label") === step.label
          : text(e) ===
            ("details" in step
              ? step.details
              : "text" in step
                ? step.text
                : step.button)),
    );
  }, JSON.stringify(step));
  target.click();
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
}
(async () => {
  try {
    // Wait for snapshot and account poll before opening controls. Startup fixtures deliberately never get data.
    await waitFor(
      () => (document.querySelector("header") ? true : undefined),
      "app",
    );
    if (!scenario.id.startsWith("startup-"))
      await waitFor(() => {
        const settings = [
          ...document.querySelectorAll<HTMLButtonElement>("footer button"),
        ].find((b) => text(b) === "Settings");
        return settings && !settings.disabled ? true : undefined;
      }, "initial snapshot");
    for (const step of scenario.steps ?? []) await runStep(step);
    await waitFor(
      () =>
        [
          document.body.textContent,
          ...[...document.querySelectorAll("input")].map((i) => i.placeholder),
        ].some((text) => text?.includes(scenario.expect))
          ? true
          : undefined,
      scenario.expect,
    );
    document.documentElement.dataset.reviewStatus = "ready";
    parent.postMessage(
      { type: "review-ready", id: scenario.id },
      location.origin,
    );
  } catch (error) {
    document.documentElement.dataset.reviewStatus = "error";
    parent.postMessage(
      { type: "review-error", id: scenario.id, error: String(error) },
      location.origin,
    );
  }
})();
