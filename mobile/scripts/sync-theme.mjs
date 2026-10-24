// The desktop palette remains the source of truth for both applications.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
const css = readFileSync(new URL("../../desktop/src/style.css", import.meta.url), "utf8");
function colors(block) {
  if (!block) throw new Error("Desktop color palette was not found");
  return Object.fromEntries([...block.matchAll(/--color-([\w-]+):\s*(#[\da-f]{6});/gi)]
    .map(([, name, value]) => [name, value]));
}
const palette = {
  light: colors(css.match(/@theme\s*\{([^}]+)\}/)?.[1]),
  dark: colors(css.match(/:root\[data-theme="dark"\]\s*\{([^}]+)\}/)?.[1]),
};
for (const name of Object.keys(palette.light)) {
  if (!palette.dark[name]) throw new Error(`Missing desktop dark color: ${name}`);
}
mkdirSync(new URL("../src/", import.meta.url), { recursive: true });
writeFileSync(new URL("../src/palette.json", import.meta.url), JSON.stringify(palette, null, 2) + "\n");
