import { createContext, useContext, useState, type PropsWithChildren } from "react";
import { useColorScheme, View } from "react-native";
import { vars } from "nativewind";
import palette from "./palette.json";
import { display } from "../modules/extend-display";

type Appearance = "system" | "light" | "dark";
const Theme = createContext({
  colors: palette.light,
  appearance: "system" as Appearance,
  setAppearance: (_value: Appearance) => {},
  dark: false,
});

export function ThemeProvider({ children }: PropsWithChildren) {
  const system = useColorScheme();
  const [appearance, setState] = useState<Appearance>(() => display.getAppearance());
  const dark = appearance === "dark" || (appearance === "system" && system === "dark");
  const colors = dark ? palette.dark : palette.light;
  const variables = vars(Object.fromEntries(Object.entries(colors).map(([name, hex]) => [
    `--color-${name}`,
    [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16)).join(" "),
  ])));
  return (
    <Theme.Provider value={{ colors, appearance, dark, setAppearance(value) {
      display.setAppearance(value); setState(value);
    } }}>
      <View className="flex-1 bg-canvas" style={variables}>{children}</View>
    </Theme.Provider>
  );
}

export const useTheme = () => useContext(Theme);
