import { Pressable, Text, TextInput, View } from "react-native";
import { useTheme } from "../theme";

export function DeviceNameEditor({ name, defaultName, custom, draft, onChange, onSave, onDiscard, busy, resetLabel }: {
  name: string; defaultName: string; custom: boolean; draft: string | undefined;
  onChange: (name: string) => void; onSave: () => void; onDiscard: () => void;
  busy: boolean; resetLabel: string;
}) {
  const { colors } = useTheme();
  const dirty = draft !== undefined && (draft.trim() === "" ? custom : draft.trim() !== name);
  return <View className="gap-3">
    <TextInput accessibilityLabel="Device name" editable={!busy} value={draft ?? name} onChangeText={onChange}
      placeholder={defaultName} placeholderTextColor={colors.muted} maxLength={100} autoCorrect={false}
      className="min-h-12 rounded-xl border border-line bg-canvas px-4 text-base text-ink" />
    <View className="flex-row flex-wrap items-center gap-2">
      <Pressable accessibilityRole="button" accessibilityLabel={resetLabel} disabled={busy || (!custom && !dirty)}
        accessibilityState={{ disabled: busy || (!custom && !dirty) }} onPress={() => onChange("")}
        className={`min-h-12 justify-center rounded-xl px-3 ${busy || (!custom && !dirty) ? "opacity-50" : "active:bg-line"}`}>
        <Text className="text-sm font-medium text-ink">{resetLabel}</Text>
      </Pressable>
      {dirty ? <>
        <Pressable accessibilityRole="button" accessibilityLabel="Discard name changes" disabled={busy} accessibilityState={{ disabled: busy }}
          onPress={onDiscard} className={`min-h-12 justify-center rounded-xl px-4 ${busy ? "opacity-50" : "active:bg-line"}`}>
          <Text className="text-sm font-medium text-ink">Discard</Text>
        </Pressable>
        <Pressable accessibilityRole="button" accessibilityLabel="Save device name" disabled={busy} accessibilityState={{ disabled: busy }}
          onPress={onSave} className={`min-h-12 justify-center rounded-xl bg-focus px-5 ${busy ? "opacity-50" : "active:opacity-80"}`}>
          <Text className="text-sm font-semibold text-canvas">{busy ? "Saving…" : "Save"}</Text>
        </Pressable>
      </> : null}
    </View>
    {dirty ? <Text accessibilityLiveRegion="polite" className="text-xs text-muted">Unsaved changes</Text> : null}
  </View>;
}
