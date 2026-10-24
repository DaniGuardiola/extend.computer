export function isReviewShortcut(event: KeyboardEvent) {
  if (
    event.defaultPrevented ||
    event.isComposing ||
    event.altKey ||
    event.ctrlKey ||
    event.metaKey ||
    event.shiftKey ||
    event.repeat ||
    !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "."].includes(
      event.key,
    )
  )
    return false;
  const target = event.target;
  return !(
    target instanceof Element &&
    target.closest(
      'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="combobox"], [role="listbox"], [role="option"], [role="slider"], [role="spinbutton"], [role="menu"], [role="menuitem"], [role="tablist"], [role="tab"], [role="radiogroup"], [aria-haspopup="listbox"], [aria-haspopup="menu"]',
    )
  );
}
