import { useEffect, useRef, useState } from "react";
import { AppState, Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { X } from "lucide-react-native";
import { display, type ReceiverDisplaySettings, type ReceiverSettingsStatus } from "../../modules/extend-display";
import { appliedDraft, chooseSize, formatConfig, matchingCompletion, observedRequest, settingsFromConfig, sizeOptions, sizeValue } from "../display/receiver-settings";

function Choices<T extends string | number>({ label, value, options, disabled, onChange }: {
  label: string; value: T; options: { value: T; label: string }[]; disabled: boolean; onChange(value: T): void;
}) {
  return <View className="gap-2">
    <Text className="text-sm font-medium text-white/70">{label}</Text>
    <View className="flex-row flex-wrap gap-2">
      {options.map(option => <Pressable key={option.value} accessibilityRole="radio" accessibilityLabel={`${label} ${option.label}`}
        accessibilityState={{ checked: option.value === value, disabled }} disabled={disabled} onPress={() => onChange(option.value)}
        className={`min-h-12 justify-center rounded-xl border px-4 ${option.value === value ? "border-white bg-white" : "border-white/20 bg-white/5"} disabled:opacity-50`}>
        <Text className={`text-sm font-medium ${option.value === value ? "text-black" : "text-white"}`}>{option.label}</Text>
      </Pressable>)}
    </View>
  </View>;
}

export function ReceiverSettingsSheet({ sessionId, onClose }: { sessionId: string; onClose(): void }) {
  const [status, setStatus] = useState<ReceiverSettingsStatus>();
  const [draft, setDraft] = useState<ReceiverDisplaySettings>();
  const [error, setError] = useState("");
  const [pollError, setPollError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const alive = useRef(false);
  const inFlight = useRef(false);
  const applying = useRef(false);
  const requestId = useRef<number | null>(null);
  const operation = useRef(0);
  useEffect(() => {
    alive.current = true;
    const refresh = async () => {
      if (inFlight.current || applying.current || AppState.currentState !== "active") return;
      inFlight.current = true;
      const version = operation.current;
      try {
        const next: ReceiverSettingsStatus = JSON.parse(await display.getDisplayReceiverSettings(sessionId));
        if (!alive.current || version !== operation.current || next.id !== sessionId) return;
        setPollError("");
        setStatus(next);
        setDraft(current => current ?? settingsFromConfig(next.applied?.config ?? next.config));
        // Closing the panel never cancels native Apply. Adopt its request when
        // reopening so completion updates the previously disabled draft too.
        requestId.current = observedRequest(next, requestId.current);
        const result = matchingCompletion(next, requestId.current);
        if (result) {
          const applied = appliedDraft(next, requestId.current);
          if (applied) setDraft(applied);
          setError(result.error ?? "");
          requestId.current = null;
        }
      } catch {
        if (alive.current && version === operation.current) setPollError("Couldn't check display settings. Close this panel and try again.");
      } finally { inFlight.current = false; }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 300);
    const foreground = AppState.addEventListener("change", state => { if (state === "active") void refresh(); });
    return () => { alive.current = false; clearInterval(timer); foreground.remove(); };
  }, [sessionId]);
  const pending = submitting || status?.pending != null;
  const disabled = pending || !status?.available;
  async function apply() {
    if (!draft || !status || disabled || applying.current || AppState.currentState !== "active") return;
    applying.current = true; operation.current += 1; setSubmitting(true); setError("");
    try {
      const request: { id: number } = JSON.parse(await display.requestDisplayReceiverSettings(sessionId, status.revision, JSON.stringify(draft)));
      if (!alive.current) return;
      requestId.current = request.id;
      setStatus(current => current ? { ...current, pending: request.id, result: null } : current);
    } catch (failure) {
      if (alive.current) setError(failure instanceof Error ? failure.message : "Couldn't request display settings.");
    } finally {
      applying.current = false;
      if (alive.current) setSubmitting(false);
    }
  }
  const frameRates = [...new Set([30, 60, 120, ...(draft ? [draft.fps] : [])])].sort((a,b) => a-b);
  const bitrates = [...new Set([8_000_000,20_000_000,40_000_000,...(draft ? [draft.bitrate] : [])])].sort((a,b) => a-b);
  const notice = pending ? "Applying settings… You can close this panel while the change continues."
    : status?.closed ? "Display ended. Reconnect to change settings."
    : status && !status.supported ? "This device doesn't support changing settings from the receiving screen."
    : status && !status.available ? "Wait for the display to connect, or reconnect if the change timed out."
    : "Changes apply to this display session.";
  return <View style={{ position:"absolute",inset:0 }} accessibilityViewIsModal>
    <Pressable style={{ position:"absolute",inset:0 }} className="bg-black/60" accessibilityLabel="Close display settings" accessibilityRole="button" onPress={onClose} />
    <SafeAreaView edges={["bottom","left","right"]} pointerEvents="box-none" style={{ position:"absolute",bottom:0,left:0,right:0,maxHeight:"85%" }}>
      <View style={{ flexShrink:1 }} className="mx-auto w-full max-w-2xl rounded-t-3xl border border-white/10 bg-zinc-950 px-5 pb-4 pt-3">
        <View className="mb-3 flex-row items-center justify-between">
          <Text accessibilityRole="header" className="text-lg font-semibold text-white">Display settings</Text>
          <Pressable onPress={onClose} accessibilityRole="button" accessibilityLabel="Close display settings" className="h-12 w-12 items-center justify-center rounded-full active:bg-white/10"><X size={21} color="white" /></Pressable>
        </View>
        <ScrollView contentContainerClassName="gap-4 pb-2" style={{ flexShrink:1 }}>
          <View className="gap-1">
            <Text className="text-xs font-medium text-white/60">Applied settings</Text>
            <Text className="text-sm text-white">{status?.applied ? formatConfig(status.applied.config) : "Waiting for the first display frame…"}</Text>
          </View>
          {draft && status ? <>
            <Choices label="Display size" value={sizeValue(draft)} options={sizeOptions(status.config,draft)} disabled={disabled} onChange={value => setDraft(current => current ? chooseSize(current,value) : current)} />
            <Choices label="Frame rate" value={draft.fps} options={frameRates.map(value => ({value,label:`${value} FPS`}))} disabled={disabled} onChange={fps => setDraft(current => current ? {...current,fps} : current)} />
            <Choices label="Video quality" value={draft.bitrate} options={bitrates.map(value => ({value,label:`${value/1_000_000} Mbps`}))} disabled={disabled} onChange={bitrate => setDraft(current => current ? {...current,bitrate} : current)} />
          </> : null}
          <Text accessibilityLiveRegion="polite" className="text-xs leading-5 text-white/65">{notice}</Text>
          {error || pollError || status?.result?.error ? <Text accessibilityRole="alert" className="text-sm leading-5 text-red-300">{error || pollError || status?.result?.error}</Text> : null}
        </ScrollView>
        <View className="mt-4 flex-row gap-3">
          <Pressable onPress={onClose} accessibilityRole="button" className="min-h-12 flex-1 items-center justify-center rounded-xl border border-white/20 px-4"><Text className="font-medium text-white">{pending ? "Close" : "Cancel"}</Text></Pressable>
          <Pressable onPress={() => void apply()} disabled={disabled || !draft} accessibilityRole="button" accessibilityState={{ disabled:disabled || !draft }} className="min-h-12 flex-1 items-center justify-center rounded-xl bg-white px-4 disabled:opacity-50"><Text className="font-medium text-black">{pending ? "Applying…" : "Apply"}</Text></Pressable>
        </View>
      </View>
    </SafeAreaView>
  </View>;
}
