import { useCallback, useEffect, useRef, useState } from "react";
import { router, useFocusEffect } from "expo-router";
import { Keyboard, Monitor, Plus, Settings2, Copy } from "lucide-react-native";
import {
  Alert,
  PermissionsAndroid,
  Platform,
  Pressable,
  ScrollView,
  Switch,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import {
  display,
  type PairedDevice,
  type ReceivingState,
} from "../../modules/extend-display";
import { Header } from "../components/Header";
import { currentStatus } from "../devices/availability";
import { useDevices } from "../devices/useDevices";
import { useTheme } from "../theme";

const labels = {
  checking: "Checking…",
  online: "Online",
  offline: "Offline",
  receiving_off: "Not accepting connections",
  unavailable: "Unavailable",
};

export default function Devices() {
  const { colors } = useTheme();
  const {
    devices,
    statuses,
    now,
    error: deviceError,
    check,
    reload,
    active,
  } = useDevices();
  const [ready, setReady] = useState(false);
  const [removed, setRemoved] = useState<{ peer: string; name: string }[]>([]);
  const [error, setError] = useState<string>();
  const [receiving, setReceiving] = useState<ReceivingState>({
    enabled: false,
    state: "stopped",
    port: 0,
  });
  const [connectionActive, setConnectionActive] = useState(false);
  const [sharingPeer, setSharingPeer] = useState<string>();
  const [receivingBusy, setReceivingBusy] = useState(false);
  const changing = useRef(false);
  const focused = useRef(false);
  useFocusEffect(
    useCallback(() => {
      focused.current = true;
      return () => {
        focused.current = false;
      };
    }, []),
  );
  useEffect(() => {
    let mounted = true;
    display
      .getEngineIdentity()
      .then(() => {
        if (mounted) {
          setReady(true);
          display.resumeReceiving();
          void display.resumeAccount().catch(() => {});
        }
      })
      .catch(() => {
        if (mounted)
          setError(
            "Couldn't prepare this device. Close and reopen the app to retry.",
          );
      });
    if (display.getLaunchSession().available) router.replace("/display");
    return () => {
      mounted = false;
    };
  }, []);
  useEffect(() => {
    if (!active || !ready) return;
    function refresh() {
      try {
        setReceiving(JSON.parse(display.getReceivingState()));
        setConnectionActive(display.hasActiveConnection());
        const sharing = JSON.parse(display.getScreenSharingState()) as {
          state: string;
          peer?: string;
        };
        setSharingPeer(
          ["consent", "connecting", "streaming"].includes(sharing.state)
            ? sharing.peer
            : undefined,
        );
      } catch {
        setError("Couldn't check this device's availability.");
      }
    }
    let pendingNotices = false;
    function refreshNotices() {
      if (pendingNotices) return;
      pendingNotices = true;
      void display.getRemovedPeers().then(value => {
        if (focused.current) setRemoved(value);
      }).catch(() => {}).finally(() => { pendingNotices = false; });
    }
    refresh(); refreshNotices();
    const timer = setInterval(() => { refresh(); refreshNotices(); }, 1000);
    return () => clearInterval(timer);
  }, [active, ready]);
  async function toggleReceiving(enabled: boolean) {
    if (changing.current) return;
    changing.current = true;
    setReceivingBusy(true);
    setError(undefined);
    try {
      if (
        enabled &&
        Platform.OS === "android" &&
        Number(Platform.Version) >= 33
      )
        await PermissionsAndroid.request(
          PermissionsAndroid.PERMISSIONS.POST_NOTIFICATIONS,
        );
      if (!focused.current) return;
      display.setReceivingEnabled(enabled);
      setReceiving(JSON.parse(display.getReceivingState()));
    } catch {
      if (focused.current)
        setError(
          "Couldn't change availability. Try again while the app is open.",
        );
    } finally {
      changing.current = false;
      if (focused.current) setReceivingBusy(false);
    }
  }
  function canStart() {
    try {
      if (!display.hasActiveConnection()) return true;
    } catch {
      /* Availability cannot be established yet. */
    }
    setError("Stop the current connection before starting another action.");
    return false;
  }
  function settings(device: PairedDevice) {
    if (!canStart()) return;
    Alert.alert(
      device.name,
      device.trustSource === "account" ? "Available through your signed-in account." : "Paired directly with this device. No account required.",
      [
        { text: "Close", style: "cancel" },
        {
          text: "Device settings",
          onPress: () => {
            if (canStart()) router.push({ pathname: "/device-settings", params: { peer: device.peer } });
          },
        },
        {
          text: "Unpair device",
          style: "destructive",
          onPress: () => {
            Alert.alert(
              "Unpair device?",
              `Disconnect ${device.name} from this tablet? Pair again to reconnect.`,
              [
                { text: "Cancel", style: "cancel" },
                {
                  text: "Unpair",
                  style: "destructive",
                  onPress: () => {
                    display
                      .forgetDevice(device.peer)
                      .then(() => {
                        if (focused.current) reload();
                      })
                      .catch(() => {
                        if (focused.current)
                          setError("Couldn't unpair this device. Retry.");
                      });
                  },
                },
              ],
            );
          },
        },
      ],
    );
  }
  const accepting = receiving.state === "available";
  return (
    <SafeAreaView className="flex-1 bg-canvas">
      <Header title="extend.computer" account />
      <ScrollView contentContainerClassName="mx-auto w-full max-w-3xl gap-5 px-5 py-5">
        <View className="flex-row items-center justify-between">
          <Text className="text-base font-medium text-ink">Devices</Text>
          <Pressable
            disabled={!ready || connectionActive}
            accessibilityRole="button"
            accessibilityLabel="Pair device"
            onPress={() => {
              if (canStart()) router.push("/add-device");
            }}
            className={`min-h-12 flex-row items-center gap-2 rounded-xl px-3 active:bg-line ${!ready || connectionActive ? "opacity-50" : ""}`}
          >
            <Plus size={18} color={colors.ink} />
            <Text className="text-sm font-medium text-ink">Pair device</Text>
          </Pressable>
        </View>
        {error || deviceError ? (
          <Text accessibilityRole="alert" className="text-sm text-muted">
            {error || deviceError}
          </Text>
        ) : null}
        {removed.map(item => (
          <View key={item.peer} className="gap-3 rounded-2xl border border-line bg-surface p-5">
            <Text className="font-medium text-ink">{item.name} is no longer paired</Text>
            <Text className="text-sm leading-6 text-muted">Pairing was removed on the other device. Pair again to reconnect.</Text>
            <Pressable accessibilityRole="button" accessibilityLabel={`Dismiss removal notice for ${item.name}`} onPress={() => {
              void display.dismissRemovedPeer(item.peer).then(() => {
                if (focused.current) setRemoved(previous => previous.filter(value => value.peer !== item.peer));
              }).catch(() => { if (focused.current) setError("Couldn't dismiss this notice. Try again."); });
            }} className="self-start rounded-xl border border-line px-4 py-3 active:bg-line">
              <Text className="font-medium text-ink">Dismiss</Text>
            </Pressable>
          </View>
        ))}
        {devices.map((device) => {
          const status = currentStatus(statuses[device.peer], now);
          const online = status.state === "online";
          const internetOnly = status.route === "internet";
          const sharing = sharingPeer === device.peer;
          return (
            <View
              key={device.peer}
              className="overflow-hidden rounded-xl border border-line bg-surface"
            >
              <View className="flex-row items-start gap-3 px-5 pb-3 pt-4">
                <Monitor
                  size={21}
                  color={online ? colors.success : colors.muted}
                />
                <View className="flex-1 gap-2">
                  <View className="flex-row flex-wrap items-center gap-2">
                    <Text className="text-base font-medium text-ink">
                      {device.name}
                    </Text>
                    <Text className="rounded-md border border-line px-1.5 py-0.5 text-xs text-muted">
                      {device.trustSource === "account" ? "Same account" : "Paired"}
                    </Text>
                  </View>
                  <View className="flex-row items-center gap-2">
                    <View
                      className={`h-1.5 w-1.5 rounded-full ${online ? "bg-success" : "bg-muted"}`}
                    />
                    <Text
                      accessibilityLiveRegion="polite"
                      className="text-sm text-muted"
                    >
                      {sharing ? "Sharing screen" : labels[status.state]}
                      {!sharing && online && status.route === "usb" ? " · USB" : ""}
                    </Text>
                  </View>
                </View>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={`Settings for ${device.name}`}
                  disabled={connectionActive}
                  onPress={() => settings(device)}
                  className="-mr-2 -mt-2 h-12 w-12 items-center justify-center rounded-xl active:bg-line"
                >
                  <Settings2 size={18} color={colors.muted} />
                </Pressable>
              </View>
              {sharing ? (
                <View className="flex-row flex-wrap gap-1 px-3 pb-3">
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={`Stop sharing with ${device.name}`}
                    onPress={() => display.stopScreenSharing()}
                    className="min-h-12 justify-center rounded-xl px-3 active:bg-line"
                  >
                    <Text className="text-sm font-medium text-ink">
                      Stop display
                    </Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    onPress={() =>
                      router.push({
                        pathname: "/share-screen",
                        params: { peer: device.peer, name: device.name },
                      })
                    }
                    className="min-h-12 justify-center rounded-xl px-3 active:bg-line"
                  >
                    <Text className="text-sm font-medium text-ink">
                      Display settings
                    </Text>
                  </Pressable>
                </View>
              ) : online ? (
                <View className="flex-row flex-wrap gap-1 px-3 pb-3">
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={`Mirror display to ${device.name}`}
                    accessibilityHint={internetOnly ? "Screen sharing needs a local connection." : undefined}
                    disabled={connectionActive || internetOnly}
                    accessibilityState={{ disabled: connectionActive || internetOnly }}
                    onPress={() => {
                      if (!internetOnly && canStart())
                        router.push({
                          pathname: "/share-screen",
                          params: { peer: device.peer, name: device.name },
                        });
                    }}
                    className={`min-h-12 flex-row items-center gap-2 rounded-xl px-3 ${connectionActive || internetOnly ? "opacity-50" : "active:bg-line"}`}
                  >
                    <Copy size={18} color={colors.ink} />
                    <Text className="text-sm font-medium text-ink">
                      Mirror display
                    </Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={`Share keyboard and mouse with ${device.name}`}
                    disabled={connectionActive}
                    onPress={() => {
                      if (canStart())
                        router.push({
                          pathname: "/control",
                          params: { peer: device.peer, name: device.name },
                        });
                    }}
                    className="min-h-12 flex-row items-center gap-2 rounded-xl px-3 active:bg-line"
                  >
                    <Keyboard size={18} color={colors.ink} />
                    <Text className="text-sm font-medium text-ink">
                      Keyboard &amp; mouse
                    </Text>
                  </Pressable>
                  {internetOnly ? <Text className="w-full px-2 pt-1 text-sm leading-6 text-muted">Screen sharing needs a local connection.</Text> : null}
                </View>
              ) : status.state === "receiving_off" ? (
                <Text className="px-5 pb-4 text-sm leading-6 text-muted">
                  Open extend.computer on {device.name} and turn on Allow
                  connections.
                </Text>
              ) : status.state !== "checking" ? (
                <View className="gap-2 px-5 pb-3">
                  {status.reason ? (
                    <Text className="text-sm leading-6 text-muted">
                      {status.reason}
                    </Text>
                  ) : null}
                  <Pressable
                    accessibilityRole="button"
                    disabled={connectionActive}
                    onPress={() => {
                      if (canStart()) check(device.peer);
                    }}
                    className="min-h-12 self-start justify-center rounded-xl px-3 active:bg-line"
                  >
                    <Text className="text-sm font-medium text-ink">
                      Check connection
                    </Text>
                  </Pressable>
                </View>
              ) : null}
            </View>
          );
        })}
        {devices.length === 0 ? (
          <View className="items-center gap-3 rounded-xl border border-line bg-surface px-7 py-12">
            <Monitor color={colors.muted} size={32} />
            <Text className="text-base font-medium text-ink">
              No paired devices
            </Text>
            <Text className="max-w-sm text-center text-sm leading-6 text-muted">
              Pair your other devices to connect securely.
            </Text>
          </View>
        ) : null}
        {connectionActive ? (
          <Text className="text-sm leading-6 text-muted">
            Stop the current connection before pairing or starting another
            connection.
          </Text>
        ) : null}
        <View className="flex-row items-center gap-3 border-t border-line pt-4">
          <View className="flex-1 gap-1">
            <Text className="text-sm font-medium text-ink">
              Allow connections
            </Text>
            <Text
              accessibilityLiveRegion="polite"
              className="text-sm text-muted"
            >
              {display.getDeviceName()} ·{" "}
              {accepting
                ? "Available"
                : receiving.state === "starting"
                  ? "Starting…"
                  : receiving.enabled
                    ? "Unavailable"
                    : "Off"}
            </Text>
          </View>
          <Switch
            accessibilityLabel="Allow connections"
            disabled={!ready || receivingBusy}
            value={receiving.enabled}
            onValueChange={(value) => void toggleReceiving(value)}
          />
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}
