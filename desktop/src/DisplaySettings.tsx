import { Select } from "./Select";
import type { DisplayConfig } from "./bridge";
import {
  connectionDisplayOptions,
  type DisplayPreferences,
} from "./displayPreferences";

export function DisplaySettings({
  value,
  onChange,
  current,
  initial,
  disabled,
}: {
  value: DisplayPreferences;
  onChange: (value: DisplayPreferences) => void;
  current: DisplayConfig;
  initial: DisplayConfig;
  disabled?: boolean;
}) {
  const supported = connectionDisplayOptions(initial, current);
  const scale = value.retina ? 2 : 1;
  const options = [
    { value: "auto", label: "Fit display" },
    ...supported.sizes
      .filter((size) => {
        const [w, h] = size.split("x").map(Number);
        return w * scale <= 4096 && h * scale <= 4096;
      })
      .map((size) => ({ value: size, label: size.replace("x", " × ") })),
  ];
  return (
    <section
      className="grid grid-cols-1 gap-4 min-[480px]:grid-cols-2"
      aria-label="Display connection settings"
    >
      <Select
        label="Stream size"
        value={value.resolution}
        options={options}
        disabled={disabled}
        onChange={(resolution) => onChange({ ...value, resolution })}
      />
      <Select
        label="Frame rate limit"
        value={String(value.fps)}
        disabled={disabled}
        options={supported.frameRates.map((fps) => ({
          value: String(fps),
          label: `${fps} FPS`,
        }))}
        onChange={(fps) => onChange({ ...value, fps: Number(fps) })}
      />
      <Select
        label="Video quality"
        value={value.quality}
        disabled={disabled}
        options={[
          { value: "responsive", label: "Lower bandwidth · 8 Mbps" },
          { value: "balanced", label: "Balanced · 20 Mbps" },
          { value: "detail", label: "Detailed · 40 Mbps" },
        ]}
        onChange={(quality) =>
          onChange({
            ...value,
            quality: quality as DisplayPreferences["quality"],
          })
        }
      />
      <div className="field">
        <span>Retina</span>
        <label
          className={`flex min-h-10 items-center justify-between gap-3 rounded-lg border border-control px-3 py-2 text-sm font-normal ${supported.retina ? "cursor-pointer" : "text-muted"}`}
        >
          <span className="leading-5">
            {supported.retina
              ? "Sharper text, uses more bandwidth"
              : "Not available for this display"}
          </span>
          <input
            type="checkbox"
            checked={supported.retina && value.retina}
            disabled={disabled || !supported.retina}
            onChange={(e) => {
              const retina = e.target.checked;
              let resolution = value.resolution;
              if (retina && resolution !== "auto") {
                const [w, h] = resolution.split("x").map(Number);
                if (w * 2 > 4096 || h * 2 > 4096) resolution = "auto";
              }
              onChange({ ...value, retina, resolution });
            }}
          />
        </label>
      </div>
    </section>
  );
}
