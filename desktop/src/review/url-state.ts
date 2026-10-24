import type { Theme } from "../Appearance";
import { scenarios } from "./scenarios.ts";

export const previewSizes = [
  { value: "fit", label: "Fit browser", width: 820, height: "100%" },
  { value: "desktop", label: "Desktop · 820 × 680", width: 820, height: 680 },
  { value: "compact", label: "Compact · 640 × 560", width: 640, height: 560 },
  { value: "wide", label: "Wide · 1100 × 760", width: 1100, height: 760 },
  { value: "narrow", label: "Narrow · 390 × 700", width: 390, height: 700 },
  { value: "compare", label: "Desktop + narrow", width: 820, height: 680 },
] as const;

export type ReviewURLState = {
  state: string;
  view: (typeof previewSizes)[number]["value"];
  theme: Theme;
  sidebar: boolean;
  filter: string;
};

export function readReviewURL(input: URL | string): ReviewURLState {
  const params = new URL(input).searchParams;
  const state = params.get("state");
  const view = previewSizes.find((size) => size.value === params.get("view"));
  const theme = params.get("theme");
  return {
    state: scenarios.some((scenario) => scenario.id === state)
      ? state!
      : scenarios[0].id,
    view: view?.value ?? "fit",
    theme: theme === "dark" || theme === "system" ? theme : "light",
    sidebar: params.get("sidebar") === "1",
    filter: params.get("filter") ?? "",
  };
}

export function writeReviewURL(
  input: URL | string,
  state: ReviewURLState,
): URL {
  const url = new URL(input);
  url.searchParams.set("state", state.state);
  url.searchParams.set("view", state.view);
  url.searchParams.set("theme", state.theme);
  url.searchParams.set("sidebar", state.sidebar ? "1" : "0");
  if (state.filter) url.searchParams.set("filter", state.filter);
  else url.searchParams.delete("filter");
  return url;
}
