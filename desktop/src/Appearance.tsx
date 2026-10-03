import { useEffect, useId, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { native } from "./bridge";
export type Theme = "system" | "light" | "dark";
export function useAppearance() {
  const [theme, setTheme] = useState<Theme>(() => {
    const saved = localStorage.getItem("extend.computer.appearance");
    return saved === "light" || saved === "dark" ? saved : "system";
  });
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        theme === "system" ? (media.matches ? "dark" : "light") : theme;
    };
    apply();
    if (native)
      void getCurrentWindow()
        .setTheme(theme === "system" ? null : theme)
        .catch(console.error);
    localStorage.setItem("extend.computer.appearance", theme);
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);
  return { theme, setTheme };
}

export function Appearance({
  theme,
  onChange,
}: {
  theme: Theme;
  onChange: (theme: Theme) => void;
}) {
  const name = useId();
  return (
    <fieldset
      aria-label="Appearance"
      className="inline-flex shrink-0 gap-1 rounded-lg border border-line bg-line/30 p-1"
    >
      {([
        { value: "system", label: "System" },
        { value: "light", label: "Light" },
        { value: "dark", label: "Dark" },
      ] as const).map((option) => (
        <label key={option.value} className="cursor-pointer">
          <input
            className="peer sr-only"
            type="radio"
            name={name}
            value={option.value}
            checked={theme === option.value}
            onChange={() => onChange(option.value)}
          />
          <span className="block min-w-16 rounded-md px-3 py-2 text-center text-xs font-medium text-muted hover:text-ink peer-focus-visible:text-ink peer-checked:bg-selected peer-checked:text-ink peer-checked:shadow-sm peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-focus">
            {option.label}
          </span>
        </label>
      ))}
    </fieldset>
  );
}
