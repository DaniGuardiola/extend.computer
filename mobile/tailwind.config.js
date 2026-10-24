const palette = require("./src/palette.json");
module.exports = {
  content: ["./src/**/*.{ts,tsx}", "./modules/**/*.tsx"],
  presets: [require("nativewind/preset")],
  darkMode: "class",
  theme: { extend: { colors: Object.fromEntries(
    Object.keys(palette.light).map((name) => [name, `rgb(var(--color-${name}) / <alpha-value>)`])
  ) } },
  plugins: [],
};
