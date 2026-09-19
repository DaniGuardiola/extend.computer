import type { ChangeEvent, KeyboardEvent } from "react";

const digits = (text: string) => text.replace(/[^0-9a-f]/gi, "").toLowerCase();
const format = (text: string) =>
  digits(text)
    .slice(0, 16)
    .match(/.{1,4}/g)
    ?.join("-") ?? "";

export function PairingCodeInput({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const update = (input: HTMLInputElement, raw: string, caret: number) => {
    const before = Math.min(digits(raw.slice(0, caret)).length, 16);
    const next = format(raw);
    // Keep the caret beside the same digit when separators are inserted/removed.
    const position = Math.min(
      next.length,
      before + Math.floor(Math.max(0, before - 1) / 4),
    );
    input.value = next;
    onChange(next);
    input.setSelectionRange(position, position);
  };
  const change = (event: ChangeEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    update(input, input.value, input.selectionStart ?? input.value.length);
  };
  const keyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const start = input.selectionStart ?? 0;
    if (
      start !== input.selectionEnd ||
      event.metaKey ||
      event.ctrlKey ||
      event.altKey
    )
      return;
    // A generated separator should never trap Backspace or Delete.
    if (event.key === "Backspace" && value[start - 1] === "-") {
      event.preventDefault();
      update(input, value.slice(0, start - 2) + value.slice(start), start - 2);
    } else if (event.key === "Delete" && value[start] === "-") {
      event.preventDefault();
      update(input, value.slice(0, start) + value.slice(start + 2), start);
    }
  };
  return (
    <input
      required
      autoComplete="off"
      autoCapitalize="off"
      spellCheck={false}
      pattern="[0-9a-f]{4}(-[0-9a-f]{4}){3}"
      title="Enter the 16-character pairing code."
      placeholder="0000-0000-0000-0000"
      value={value}
      onChange={change}
      onKeyDown={keyDown}
    />
  );
}
