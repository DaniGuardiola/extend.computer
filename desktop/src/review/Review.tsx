import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Appearance, type Theme } from "../Appearance";
import {
  ArrowLeft,
  ArrowRight,
  RotateCcw,
  List,
  CheckCheck,
} from "lucide-react";
import { scenarios } from "./scenarios";
import { isReviewShortcut } from "./shortcuts";
import {
  previewSizes as sizes,
  readReviewURL,
  writeReviewURL,
} from "./url-state";
import { useReviewURL } from "./useReviewURL";
import "../style.css";
import "./review.css";

const groups = [...new Set(scenarios.map((s) => s.group))];
function Review() {
  const [settings, updateSettings] = useReviewURL();
  const { theme, sidebar: list, filter } = settings;
  const index = scenarios.findIndex(
    (scenario) => scenario.id === settings.state,
  );
  const size = sizes.find((size) => size.value === settings.view)!;
  const comparing = size.value === "compare";
  const [availableWidth, setAvailableWidth] = useState(0);
  // Scale the previews visually while preserving each iframe's actual viewport.
  const comparisonScale = Math.min(
    1,
    Math.max(0.1, (availableWidth - 24) / 1214),
  );
  const [reset, setReset] = useState(0);
  const [status, setStatus] = useState("Loading screen…");
  const [auditing, setAuditing] = useState(false);
  const [report, setReport] = useState("");
  const frame = useRef<HTMLIFrameElement>(null);
  const narrowFrame = useRef<HTMLIFrameElement>(null);
  const frameResults = useRef<
    Partial<Record<"desktop" | "narrow", string | null>>
  >({});
  const stage = useRef<HTMLElement>(null);
  useEffect(() => {
    const observer = new ResizeObserver(([entry]) =>
      setAvailableWidth(entry.contentRect.width),
    );
    observer.observe(stage.current!);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    stage.current?.scrollTo(0, 0);
  }, [size.value, list]);
  const auditResult = useRef<((result: string | null) => void) | null>(null);
  const scenario = scenarios[index];
  // Stable src: theme updates via postMessage preserve entered form values and dialog state.
  const [frameTheme, setFrameTheme] = useState<Theme>(theme);
  const resetStatus = () => {
    frameResults.current = {};
    setStatus("Loading screen…");
  };
  const sendTheme = (value: Theme) => {
    for (const preview of [frame.current, narrowFrame.current])
      preview?.contentWindow?.postMessage(
        { type: "review-theme", theme: value },
        location.origin,
      );
  };
  const go = (next: number) => {
    resetStatus();
    setFrameTheme(theme);
    updateSettings({ state: scenarios[next].id });
  };
  const changeSize = (next: (typeof sizes)[number]) => {
    if ((next.value === "compare") !== comparing) {
      resetStatus();
      setFrameTheme(theme);
      setReset((n) => n + 1);
    }
    updateSettings({ view: next.value });
  };
  useEffect(() => {
    const restore = () => {
      const next = readReviewURL(location.href);
      if (
        next.state !== scenario.id ||
        (next.view === "compare") !== comparing
      ) {
        resetStatus();
        setFrameTheme(next.theme);
        setReset((n) => n + 1);
      }
    };
    window.addEventListener("popstate", restore);
    return () => window.removeEventListener("popstate", restore);
  }, [scenario.id, comparing]);
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        theme === "system" ? (media.matches ? "dark" : "light") : theme;
      sendTheme(theme);
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      const source =
        event.source === frame.current?.contentWindow
          ? "desktop"
          : comparing && event.source === narrowFrame.current?.contentWindow
            ? "narrow"
            : null;
      if (
        event.origin !== location.origin ||
        !source ||
        event.data?.id !== scenario.id
      )
        return;
      if (event.data.type === "review-app-theme") {
        const value = event.data.theme;
        if (value === "light" || value === "dark" || value === "system") {
          updateSettings({ theme: value });
        }
        return;
      }
      if (
        event.data.type === "review-ready" ||
        event.data.type === "review-error"
      ) {
        const error =
          event.data.type === "review-error" ? event.data.error : null;
        frameResults.current[source] = error;
        // Apply theme again after async app mount, without resetting the screen.
        sendTheme(theme);
        const expected = comparing
          ? (["desktop", "narrow"] as const)
          : (["desktop"] as const);
        const errors = expected
          .filter((view) => frameResults.current[view])
          .map((view) => `${view}: ${frameResults.current[view]}`);
        const ready = expected.every(
          (view) => frameResults.current[view] !== undefined,
        );
        setStatus(
          errors.join(" · ") ||
            (ready ? "Ready · interactive" : "Loading other preview…"),
        );
        if (ready) {
          stage.current?.scrollTo(0, 0);
          auditResult.current?.(errors.length ? errors.join(" · ") : null);
          auditResult.current = null;
        }
      }
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, [scenario.id, theme, comparing]);
  const changeTheme = (value: Theme) => {
    updateSettings({ theme: value });
  };
  useEffect(() => {
    const shortcut = (key: string) => {
      if (auditing) return;
      if (key === "ArrowLeft" || key === "ArrowRight") {
        const next = Math.max(
          0,
          Math.min(
            scenarios.length - 1,
            index + (key === "ArrowLeft" ? -1 : 1),
          ),
        );
        if (next !== index) go(next);
      } else if (key === "ArrowUp" || key === "ArrowDown") {
        const current = sizes.findIndex((s) => s.value === size.value);
        changeSize(
          sizes[
            (current + (key === "ArrowUp" ? -1 : 1) + sizes.length) %
              sizes.length
          ],
        );
      } else if (key === ".") {
        const dark =
          theme === "system"
            ? matchMedia("(prefers-color-scheme: dark)").matches
            : theme === "dark";
        changeTheme(dark ? "light" : "dark");
      }
    };
    const keydown = (event: KeyboardEvent) => {
      if (!isReviewShortcut(event) || auditing) return;
      event.preventDefault();
      shortcut(event.key);
    };
    const message = (event: MessageEvent) => {
      if (
        event.origin === location.origin &&
        (event.source === frame.current?.contentWindow ||
          event.source === narrowFrame.current?.contentWindow) &&
        event.data?.type === "review-shortcut" &&
        event.data?.id === scenario.id
      )
        shortcut(event.data.key);
    };
    window.addEventListener("keydown", keydown);
    window.addEventListener("message", message);
    return () => {
      window.removeEventListener("keydown", keydown);
      window.removeEventListener("message", message);
    };
  }, [index, size.value, theme, auditing]);
  async function checkCoverage() {
    setAuditing(true);
    setReport("Checking every screen…");
    const errors: string[] = [];
    for (let i = 0; i < scenarios.length; i++) {
      let timeout: ReturnType<typeof setTimeout>;
      const result = new Promise<string | null>((resolve) => {
        timeout = setTimeout(() => {
          auditResult.current = null;
          resolve("Preview did not respond");
        }, 12000);
        auditResult.current = resolve;
      });
      go(i);
      setReset((n) => n + 1);
      const error = await result;
      clearTimeout(timeout!);
      if (error) errors.push(`${scenarios[i].id}: ${error}`);
      setReport(
        `${i + 1}/${scenarios.length} checked · ${errors.length} failed`,
      );
    }
    setAuditing(false);
    setReport(
      errors.length
        ? errors.join("\n")
        : `${scenarios.length}/${scenarios.length} screens verified`,
    );
    go(0);
  }
  return (
    <div className="review-shell">
      <header className="review-heading" data-ui-review-controls>
        <div>
          <span className="review-kicker">EXTEND.COMPUTER / UI REVIEW</span>
          <h1>
            {scenario.group} <span>/</span> {scenario.title}
          </h1>
          <p>{scenario.note}</p>
        </div>
        <div className="review-position">
          <span>{String(index + 1).padStart(2, "0")}</span> / {scenarios.length}
          <small role="status" data-testid="review-status">
            {status}
          </small>
        </div>
      </header>
      <div className="review-workspace">
        {list && (
          <aside
            className="review-catalog"
            data-ui-review-controls
            aria-label="Screen catalogue"
          >
            <input
              aria-label="Filter screens"
              placeholder="Find a screen…"
              value={filter}
              onChange={(e) => updateSettings({ filter: e.target.value })}
            />
            {groups.map((group) => (
              <section key={group}>
                <h2>{group}</h2>
                {scenarios.map((s, i) =>
                  s.group === group &&
                  `${s.title} ${s.note}`
                    .toLowerCase()
                    .includes(filter.toLowerCase()) ? (
                    <button
                      key={s.id}
                      disabled={auditing}
                      aria-current={i === index ? "step" : undefined}
                      onClick={() => go(i)}
                    >
                      <span>{String(i + 1).padStart(2, "0")}</span>
                      {s.title}
                    </button>
                  ) : null,
                )}
              </section>
            ))}
          </aside>
        )}
        <main ref={stage} className="review-stage">
          <div className={comparing ? "review-comparison" : "review-single"}>
            <section
              className="review-preview"
              aria-label={comparing ? "Desktop preview" : "App preview"}
              data-ui-review-preview
              style={{
                width: comparing
                  ? 822 * comparisonScale
                  : size.value === "fit"
                    ? "100%"
                    : size.width,
                height: comparing ? undefined : size.height,
              }}
            >
              {comparing && (
                <p className="review-preview-label">
                  Desktop <span>820 × 680</span>
                </p>
              )}
              <div
                style={{ height: comparing ? 682 * comparisonScale : "100%" }}
              >
                <div
                  className="review-window"
                  style={{
                    width: comparing ? 822 : "100%",
                    height: comparing ? 682 : "100%",
                    transform: comparing
                      ? `scale(${comparisonScale})`
                      : undefined,
                    transformOrigin: "top left",
                  }}
                >
                  <iframe
                    ref={frame}
                    key={`${scenario.id}-${reset}`}
                    title={
                      comparing
                        ? "Desktop app preview"
                        : "Interactive app preview"
                    }
                    src={`/review-frame.html?state=${scenario.id}&theme=${frameTheme}`}
                  />
                </div>
              </div>
            </section>
            {comparing && (
              <section
                className="review-preview"
                aria-label="Narrow preview"
                data-ui-review-preview
                style={{ width: 392 * comparisonScale }}
              >
                <p className="review-preview-label">
                  Narrow <span>390 × 700</span>
                </p>
                <div style={{ height: 702 * comparisonScale }}>
                  <div
                    className="review-window"
                    style={{
                      width: 392,
                      height: 702,
                      transform: `scale(${comparisonScale})`,
                      transformOrigin: "top left",
                    }}
                  >
                    <iframe
                      ref={narrowFrame}
                      key={`${scenario.id}-${reset}`}
                      title="Narrow app preview"
                      src={`/review-frame.html?state=${scenario.id}&theme=${frameTheme}`}
                    />
                  </div>
                </div>
              </section>
            )}
          </div>
        </main>
      </div>
      <footer
        className="review-controls"
        data-ui-review-controls
        aria-label="Temporary review controls"
      >
        <div className="review-navigation">
          <button
            className="button quiet"
            aria-label="Toggle screen catalogue"
            aria-expanded={list}
            onClick={() => updateSettings({ sidebar: !list })}
          >
            <List size={16} />
          </button>
          <button
            className="button quiet"
            aria-label="Previous screen"
            aria-keyshortcuts="ArrowLeft"
            disabled={index === 0 || auditing}
            onClick={() => go(index - 1)}
          >
            <ArrowLeft size={16} />
          </button>
          <select
            aria-label="Review screen"
            disabled={auditing}
            value={scenario.id}
            onChange={(e) =>
              go(scenarios.findIndex((s) => s.id === e.target.value))
            }
          >
            {groups.map((group) => (
              <optgroup key={group} label={group}>
                {scenarios.map((s, i) =>
                  s.group === group ? (
                    <option key={s.id} value={s.id}>
                      {i + 1}. {s.title}
                    </option>
                  ) : null,
                )}
              </optgroup>
            ))}
          </select>
          <button
            className="button primary"
            aria-label="Next screen"
            aria-keyshortcuts="ArrowRight"
            disabled={index === scenarios.length - 1 || auditing}
            onClick={() => go(index + 1)}
          >
            <ArrowRight size={16} />
          </button>
          <button
            className="button quiet"
            aria-label="Reset screen"
            disabled={auditing}
            onClick={() => {
              resetStatus();
              setFrameTheme(theme);
              setReset((n) => n + 1);
            }}
          >
            <RotateCcw size={15} />
          </button>
          <span className="review-shortcuts">
            ← → screens · ↑ ↓ layout · . theme
          </span>
        </div>
        <div className="review-options">
          <select
            aria-label="Preview window size"
            aria-keyshortcuts="ArrowUp ArrowDown"
            disabled={auditing}
            value={size.value}
            onChange={(e) =>
              changeSize(sizes.find((s) => s.value === e.target.value)!)
            }
          >
            {sizes.map((s) => (
              <option key={s.value} value={s.value}>
                {s.label}
              </option>
            ))}
          </select>
          <span className="review-separator" />
          <span className="review-theme-label">Preview theme</span>
          <Appearance theme={theme} onChange={changeTheme} />
          <button
            className="button quiet"
            disabled={auditing}
            onClick={() => void checkCoverage()}
            title="Check that every fixture opens its intended screen"
          >
            <CheckCheck size={16} />
            Check all
          </button>
        </div>
        {report && (
          <output className="review-report" data-testid="coverage-report">
            {report}
          </output>
        )}
      </footer>
    </div>
  );
}
if (import.meta.env.MODE !== "ui-review") {
  createRoot(document.getElementById("root")!).render(
    <p>Run npm run ui:review to open the review.</p>,
  );
} else {
  const settings = readReviewURL(location.href);
  history.replaceState(
    history.state,
    "",
    writeReviewURL(location.href, settings),
  );
  document.documentElement.dataset.theme =
    settings.theme === "system"
      ? matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light"
      : settings.theme;
  createRoot(document.getElementById("root")!).render(<Review />);
}
