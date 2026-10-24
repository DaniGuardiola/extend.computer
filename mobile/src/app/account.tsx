import { useCallback, useRef, useState } from "react";
import { useFocusEffect } from "expo-router";
import { Alert, AppState, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { display, type AccountState, type AccountDevice } from "../../modules/extend-display";
import { Header } from "../components/Header";

function AccountButton({ label, onPress, disabled = false, destructive = false }: {
  label: string; onPress: () => void; disabled?: boolean; destructive?: boolean;
}) {
  return <Pressable accessibilityRole="button" accessibilityLabel={label} accessibilityState={{ disabled }} disabled={disabled} onPress={onPress}
    className={`min-h-12 justify-center rounded-xl px-4 ${disabled ? "opacity-50" : "active:bg-line"}`}>
    <Text className={`font-medium ${destructive ? "text-red-600" : "text-ink"}`}>{label}</Text>
  </Pressable>;
}

export default function Account() {
  const [state, setState] = useState<AccountState>();
  const [error, setError] = useState<string>();
  const [readError, setReadError] = useState<string>();
  const [working, setWorking] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [server, setServer] = useState("");
  const generation = useRef(0);
  const pending = useRef(false);
  const stateRead = useRef(false);
  const focused = useRef(false);
  useFocusEffect(useCallback(() => {
    const epoch = ++generation.current;
    focused.current = true; setWorking(pending.current); setError(undefined);
    let timer: ReturnType<typeof setTimeout>;
    let stopped = false;
    const valid = () => !stopped && generation.current === epoch;
    const schedule = () => { clearTimeout(timer); if (valid()) timer = setTimeout(() => void refresh(), 1000); };
    const refresh = async () => {
      if (!valid()) return;
      if (AppState.currentState === "active" && !stateRead.current) {
        stateRead.current = true;
        try {
          const next = JSON.parse(await display.getAccountState()) as AccountState;
          if (valid()) { setState(next); setReadError(undefined); }
        } catch { if (valid()) setReadError("Couldn't check your account. Close and reopen this screen to retry."); }
        finally { stateRead.current = false; }
      }
      schedule();
    };
    void display.resumeAccount().catch(() => {});
    void refresh();
    const lifecycle = AppState.addEventListener("change", value => { if (value === "active") void refresh(); });
    return () => { stopped = true; focused.current = false; generation.current++; clearTimeout(timer); lifecycle.remove(); };
  }, []));
  async function act(operation: () => Promise<unknown>) {
    if (pending.current) return;
    const epoch = generation.current;
    pending.current = true; setWorking(true); setError(undefined);
    try {
      await operation();
      const next = JSON.parse(await display.getAccountState()) as AccountState;
      if (generation.current === epoch) setState(next);
    } catch (failed) {
      if (generation.current === epoch) setError(failed instanceof Error ? failed.message : "Couldn't complete the account request. Try again.");
    } finally { pending.current = false; if (focused.current) setWorking(false); }
  }
  function remove(device: AccountDevice) {
    Alert.alert("Remove device?", `Remove ${device.name} from your account? It must sign in again to reconnect through your account.`, [
      { text: "Cancel", style: "cancel" },
      { text: "Remove", style: "destructive", onPress: () => void act(() => display.removeAccountDevice(device.id)) },
    ]);
  }
  const busy = working || !!state?.busy;
  const signingIn = !!state?.browser_pending || !!state?.native_pending;
  return <SafeAreaView className="flex-1 bg-canvas">
    <Header title="Account" back />
    <ScrollView contentContainerClassName="mx-auto w-full max-w-xl gap-5 px-5 py-6" keyboardShouldPersistTaps="handled">
      {(error || readError || state?.error) && <Text accessibilityRole="alert" className="text-sm leading-6 text-red-600">{error || readError || state?.error}</Text>}
      {!state?.loaded && !state?.storage_error ? <Text accessibilityLiveRegion="polite" className="text-sm text-muted">Loading account…</Text> : state?.email ? <>
        <View className="gap-2 rounded-2xl border border-line bg-surface p-5">
          <Text className="text-base font-semibold text-ink">{state.email}</Text>
          <Text className="text-sm leading-6 text-muted">Devices signed in to this account can connect without pairing codes. Connections currently require the same local network.</Text>
          <View className="flex-row flex-wrap"><AccountButton label="Account settings" disabled={busy} onPress={() => void act(() => display.openAccountPage("account"))} /><AccountButton label="Refresh" disabled={busy} onPress={() => void act(() => display.refreshAccount())} /></View>
        </View>
        <Text className="text-sm font-medium text-muted">Account devices</Text>
        {!state.devices.length && <Text className="text-sm leading-6 text-muted">Preparing this device…</Text>}
        {state.devices.map(device => <View key={device.id} className="gap-2 rounded-2xl border border-line bg-surface p-5">
          <Text className="text-base font-medium text-ink">{device.name}{device.id === state.device_id ? " · This device" : ""}</Text>
          <Text className="text-sm text-muted">{device.platform} · {device.key_verified ? "Identity verified" : "Identity not verified"}{device.online && state.directory_fresh ? " · Recently active" : ""}</Text>
          <View className="self-start"><AccountButton label={`Remove ${device.name}`} onPress={() => remove(device)} disabled={busy} destructive /></View>
        </View>)}
        <AccountButton label="Sign out" onPress={() => Alert.alert("Sign out?", "Account connections will close. Devices paired directly remain paired.", [
          { text: "Cancel", style: "cancel" }, { text: "Sign out", style: "destructive", onPress: () => void act(() => display.logoutAccount()) },
        ])} disabled={busy} destructive />
      </> : signingIn ? <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <Text accessibilityLiveRegion="polite" className="text-base font-medium text-ink">{state?.browser_pending ? "Continue in your browser" : "Complete sign-in"}</Text>
        <Text className="text-sm leading-6 text-muted">{state?.browser_pending ? "Return here when you finish. Sign-in expires after five minutes." : "Use the sign-in form. Complete sign-in within five minutes."}</Text>
        <AccountButton label="Cancel sign-in" onPress={() => void act(() => display.cancelAccountSignIn())} disabled={working} />
      </View> : <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <Text className="text-base font-semibold text-ink">Connect your devices</Text>
        <Text className="text-sm leading-6 text-muted">Sign in on each device to connect without pairing codes. Direct pairing remains available without an account.</Text>
        {state?.storage_error ? <AccountButton label="Sign out locally" disabled={busy} onPress={() => void act(() => display.logoutAccount())} /> : <>
          <Pressable accessibilityRole="button" accessibilityLabel="Sign in with browser" accessibilityState={{ disabled: busy }} disabled={busy} onPress={() => void act(() => display.startAccountBrowser())}
            className={`min-h-12 items-center justify-center rounded-xl bg-focus px-4 ${busy ? "opacity-50" : "active:opacity-80"}`}><Text className="font-semibold text-canvas">Sign in with browser</Text></Pressable>
          <AccountButton label="Sign in with password" disabled={busy} onPress={() => void act(() => display.startAccountPassword())} />
          <View className="flex-row flex-wrap"><AccountButton label="Create account" disabled={busy} onPress={() => void act(() => display.openAccountPage("signup"))} /><AccountButton label="Recover account" disabled={busy} onPress={() => void act(() => display.openAccountPage("recover"))} /></View>
        </>}
      </View>}
      {state && <>
        <Text className="text-xs text-muted">{state.server}</Text>
        <AccountButton label={advanced ? "Close server settings" : "Use another server"} onPress={() => { setServer(state.server); setAdvanced(value => !value); }} disabled={busy || signingIn || !!state.email} />
        {advanced && <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
          <Text className="text-sm font-medium text-ink">Account server</Text>
          <TextInput accessibilityLabel="Account server URL" value={server} onChangeText={setServer} autoCapitalize="none" autoCorrect={false} keyboardType="url"
            className="min-h-12 rounded-xl border border-line bg-canvas px-4 text-ink" />
          <AccountButton label="Save server" onPress={() => void act(async () => { await display.configureAccount(server); })} disabled={busy || signingIn || !!state.email} />
        </View>}
      </>}
    </ScrollView>
  </SafeAreaView>;
}
