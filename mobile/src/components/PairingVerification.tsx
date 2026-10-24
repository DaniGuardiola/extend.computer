import { Pressable, Text, View } from "react-native";
import { validSymbols, verificationAlphabet } from "../pairing/symbols";

export function PairingVerification({
  symbols,
  busy,
  respond,
}: {
  symbols?: number[];
  busy: boolean;
  respond(allow: boolean): void;
}) {
  const valid = validSymbols(symbols);
  return (
    <View className="gap-4 rounded-xl border border-line bg-surface p-5">
      <Text className="text-lg font-semibold text-ink">
        Compare these symbols
      </Text>
      <Text className="text-sm leading-6 text-muted">
        Only confirm if all eight symbols match on both devices, in the same
        order.
      </Text>
      {valid ? (
        <View
          accessibilityLabel="Verification symbols in order"
          className="flex-row flex-wrap gap-y-4"
        >
          {symbols.map((index, position) => {
            const [emoji, label] = verificationAlphabet[index];
            return (
              <View
                key={position}
                accessible
                className="w-1/4 items-center gap-1"
                accessibilityLabel={`${position + 1}. ${label}`}
              >
                <Text
                  accessibilityElementsHidden
                  importantForAccessibility="no"
                  className="text-3xl"
                >
                  {emoji}
                </Text>
                <Text className="text-xs text-muted">
                  {position + 1}. {label}
                </Text>
              </View>
            );
          })}
        </View>
      ) : (
        <Text accessibilityRole="alert" className="text-sm text-muted">
          Couldn’t verify these symbols. Cancel pairing and retry.
        </Text>
      )}
      <View className="flex-row gap-3">
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Symbols differ"
          disabled={busy}
          onPress={() => respond(false)}
          className="min-h-12 flex-1 items-center justify-center rounded-xl bg-canvas px-3 active:bg-line"
        >
          <Text className="text-sm font-medium text-ink">They differ</Text>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Symbols match"
          accessibilityState={{ disabled: busy || !valid }}
          disabled={busy || !valid}
          onPress={() => respond(true)}
          className={`min-h-12 flex-1 items-center justify-center rounded-xl bg-ink px-3 ${busy || !valid ? "opacity-50" : "active:opacity-80"}`}
        >
          <Text className="text-sm font-medium text-canvas">They match</Text>
        </Pressable>
      </View>
    </View>
  );
}
