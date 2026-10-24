import { useState } from "react";
import { Pressable, ScrollView, Text, View } from "react-native";
import type { EditingShortcut } from "../../modules/extend-display";

const actions: { action: EditingShortcut; label: string }[] = [
  { action: "select_all", label: "Select all" },
  { action: "copy", label: "Copy" },
  { action: "paste", label: "Paste" },
  { action: "undo", label: "Undo" },
];

export function EditingShortcuts({ token, inverse = false, onAction }: {
  token?: string | null;
  inverse?: boolean;
  onAction(action: EditingShortcut): Promise<boolean>;
}) {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ token: string; message: string }>();
  async function run(action: EditingShortcut) {
    if (!token || busy) return;
    const requested = token;
    setBusy(true);
    setFailure(undefined);
    try {
      if (!await onAction(action)) throw new Error("Shortcut unavailable");
    } catch (error) {
      setFailure({ token: requested, message:
        String(error).includes("Release physical keys")
          ? "Release physical keys before using shortcuts."
          : "Keyboard input changed. Try again." });
    } finally {
      setBusy(false);
    }
  }
  const disabled = !token || busy;
  return <View className="gap-1">
    <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerClassName="flex-row gap-1">
      {actions.map(({ action, label }) => <Pressable key={action}
        accessibilityRole="button" accessibilityLabel={`${label} on remote device`}
        accessibilityState={{ disabled }} disabled={disabled} onPress={() => void run(action)}
        className={`min-h-12 justify-center rounded-xl px-3 ${inverse ? "active:bg-white/20" : "border border-line bg-surface active:bg-line"} ${disabled ? "opacity-50" : ""}`}>
        <Text className={inverse ? "text-sm text-white" : "text-sm text-ink"}>{label}</Text>
      </Pressable>)}
    </ScrollView>
    {failure && failure.token === token ? <Text accessibilityRole="alert" className={inverse ? "text-xs text-white/70" : "text-xs text-muted"}>{failure.message}</Text> : null}
  </View>;
}
