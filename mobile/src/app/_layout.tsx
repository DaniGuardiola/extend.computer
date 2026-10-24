import "../../global.css";
import { Stack, router, usePathname } from "expo-router";
import { useEffect } from "react";
import { AppState } from "react-native";
import { display } from "../../modules/extend-display";
import { StatusBar } from "expo-status-bar";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { ThemeProvider, useTheme } from "../theme";

function Screens() {
  const { colors, dark } = useTheme();
  const pathname = usePathname();
  useEffect(() => {
    if (pathname === "/display") return;
    let navigating = false;
    const timer = setInterval(() => {
      if (navigating || AppState.currentState !== "active") return;
      try {
        const state = JSON.parse(display.getMediaState()) as { state: string };
        if (state.state === "offered") { navigating = true; router.replace("/display"); }
      } catch { /* Native availability can be restarting independently. */ }
    }, 300);
    return () => clearInterval(timer);
  }, [pathname]);
  return <>
    <StatusBar style={dark ? "light" : "dark"} />
    <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: colors.canvas } }}>
      <Stack.Screen name="index" />
      <Stack.Screen name="settings" />
      <Stack.Screen name="control" />
      <Stack.Screen name="display" options={{ animation: "fade", gestureEnabled: false }} />
    </Stack>
  </>;
}

export default function RootLayout() {
  return <SafeAreaProvider><ThemeProvider><Screens /></ThemeProvider></SafeAreaProvider>;
}
