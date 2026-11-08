import assert from "node:assert/strict";
import test from "node:test";
import { nextZoom, parseZoom, zoomAction, zoomLevels } from "../src/zoom.ts";

const key = (key, modifiers = {}) => ({
  key,
  code: "",
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  ...modifiers,
});

test("Command zooms on a Mac and Control elsewhere", () => {
  const command = { metaKey: true };
  const control = { ctrlKey: true };
  for (const [pressed, action] of [
    ["=", "in"],
    ["+", "in"],
    ["-", "out"],
    ["_", "out"],
    ["0", "reset"],
  ]) {
    assert.equal(zoomAction(key(pressed, command), true), action, pressed);
    assert.equal(zoomAction(key(pressed, control), false), action, pressed);
    // The other platform's modifier, or none, leaves the key alone.
    assert.equal(zoomAction(key(pressed, control), true), null, pressed);
    assert.equal(zoomAction(key(pressed, command), false), null, pressed);
    assert.equal(zoomAction(key(pressed), true), null, pressed);
  }
  // Option and extra modifiers keep their own meaning.
  assert.equal(
    zoomAction(key("=", { metaKey: true, altKey: true }), true),
    null,
  );
  assert.equal(
    zoomAction(key("=", { metaKey: true, ctrlKey: true }), true),
    null,
  );
  assert.equal(zoomAction(key("1", { metaKey: true }), true), null);
  // The number pad works too.
  for (const [code, action] of [
    ["NumpadAdd", "in"],
    ["NumpadSubtract", "out"],
    ["Numpad0", "reset"],
  ])
    assert.equal(
      zoomAction({ ...key("Unidentified", { ctrlKey: true }), code }, false),
      action,
    );
});

test("zoom steps stay within sane bounds", () => {
  assert.deepEqual(zoomLevels, [0.8, 0.9, 1, 1.1, 1.25, 1.5]);
  assert.equal(nextZoom(1, "in"), 1.1);
  assert.equal(nextZoom(1.1, "in"), 1.25);
  assert.equal(nextZoom(1.5, "in"), 1.5);
  assert.equal(nextZoom(1, "out"), 0.9);
  assert.equal(nextZoom(0.8, "out"), 0.8);
  assert.equal(nextZoom(1.25, "reset"), 1);
  // A level between steps moves to the neighbouring step.
  assert.equal(nextZoom(1.2, "in"), 1.25);
  assert.equal(nextZoom(1.2, "out"), 1.1);
});

test("a saved zoom must be one of the steps", () => {
  assert.equal(parseZoom("1.25"), 1.25);
  assert.equal(parseZoom("0.8"), 0.8);
  for (const saved of [null, "", "3", "0", "-1", "1.3", "large", "NaN"])
    assert.equal(parseZoom(saved), 1, String(saved));
});
