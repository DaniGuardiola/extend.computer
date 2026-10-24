import { useEffect, useState } from "react";
import type { Theme } from "../Appearance";
export { Appearance, type Theme } from "../Appearance";

// The external toggle updates the same hook state as the app's settings picker.
export function useAppearance() {
  const [theme, setTheme] = useState<Theme>(() => {
    const value = new URLSearchParams(location.search).get("theme");
    return value === "system" || value === "dark" ? value : "light";
  });
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        theme === "system" ? (media.matches ? "dark" : "light") : theme;
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (
        event.origin !== location.origin ||
        event.source !== parent ||
        event.data?.type !== "review-theme"
      )
        return;
      const value = event.data.theme;
      if (value === "light" || value === "dark" || value === "system")
        setTheme(value);
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, []);
  return {
    theme,
    setTheme: (value: Theme) => {
      setTheme(value);
      parent.postMessage(
        {
          type: "review-app-theme",
          id: new URLSearchParams(location.search).get("state"),
          theme: value,
        },
        location.origin,
      );
    },
  };
}
