import "../../global.css";
import { Stack, router, usePathname } from "expo-router";
import { useEffect } from "react";
import { AppState } from "react-native";
import { display } from "../../modules/extend-display";
import { StatusBar } from "expo-status-bar";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { ThemeProvider, useTheme } from "../theme";
import { isIOS } from "../platform";

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
    {/* iOS uses view-controller-based status bar appearance (the app-level
        API is ignored under the UIScene lifecycle): the default style follows
        the window's appearance, and Immersive hides it on the display screen. */}
    {!isIOS && <StatusBar style={dark ? "light" : "dark"} />}
    <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: colors.canvas } }}>
      <Stack.Screen name="index" />
      <Stack.Screen name="settings" />
      {/* iOS 26 pops on a full-width swipe by default; that would steal trackpad swipes. Edge swipe still goes back. */}
      <Stack.Screen name="control" options={{ fullScreenGestureEnabled: false }} />
      <Stack.Screen name="display" options={{ animation: "fade", gestureEnabled: false }} />
    </Stack>
  </>;
}

export default function RootLayout() {
  return <SafeAreaProvider><ThemeProvider><Screens /></ThemeProvider></SafeAreaProvider>;
}
