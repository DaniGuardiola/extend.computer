import * as Ariakit from "@ariakit/react";
import { Check, ChevronDown } from "lucide-react";
type Props = {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
  compact?: boolean;
  disabled?: boolean;
};
export function Select({
  label,
  value,
  onChange,
  options,
  compact,
  disabled,
}: Props) {
  return (
    <Ariakit.SelectProvider
      value={value}
      setValue={(v) => {
        if (typeof v === "string") onChange(v);
      }}
    >
      <div className={compact ? "flex items-center gap-2" : "field"}>
        <Ariakit.SelectLabel className={compact ? "sr-only" : "text-xs"}>
          {label}
        </Ariakit.SelectLabel>
        <Ariakit.Select
          disabled={disabled}
          className={`select-button ${compact ? "!min-w-28 !py-1.5" : ""}`}
        >
          <span>{options.find((o) => o.value === value)?.label || value}</span>
          <ChevronDown size={14} />
        </Ariakit.Select>
      </div>
      <Ariakit.SelectPopover
        portal
        gutter={5}
        sameWidth
        className="select-popover"
      >
        {options.map((o) => (
          <Ariakit.SelectItem
            key={o.value}
            value={o.value}
            className="select-option"
          >
            <span>{o.label}</span>
            {o.value === value && <Check size={14} />}
          </Ariakit.SelectItem>
        ))}
      </Ariakit.SelectPopover>
    </Ariakit.SelectProvider>
  );
}
