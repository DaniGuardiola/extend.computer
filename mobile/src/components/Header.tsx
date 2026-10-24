import { router } from "expo-router";
import { ArrowLeft, LogIn, Settings } from "lucide-react-native";
import { Pressable, Text, View } from "react-native";
import { useTheme } from "../theme";

export function Header({ title, back = false, account = false }: { title: string; back?: boolean; account?: boolean }) {
  const { colors } = useTheme();
  return (
    <View className="flex-row items-center justify-between px-5 py-3">
      <View className="flex-row items-center gap-3">
        {back && <Pressable onPress={() => router.back()} accessibilityLabel="Back" accessibilityRole="button"
          className="h-12 w-12 items-center justify-center rounded-full active:bg-line">
          <ArrowLeft size={22} color={colors.ink} />
        </Pressable>}
        <Text className="text-xl font-semibold text-ink">{title}</Text>
      </View>
      {!back && <View className="flex-row items-center gap-1">{account && <Pressable onPress={() => router.push("/account")} accessibilityLabel="Account" accessibilityRole="button"
        className="min-h-12 flex-row items-center gap-2 rounded-full px-3 active:bg-line"><LogIn size={18} color={colors.ink} /><Text className="text-sm text-ink">Account</Text></Pressable>}<Pressable onPress={() => router.push("/settings")} accessibilityLabel="Settings" accessibilityRole="button"
        className="h-12 w-12 items-center justify-center rounded-full active:bg-line">
        <Settings size={22} color={colors.ink} />
      </Pressable></View>}
    </View>
  );
}
