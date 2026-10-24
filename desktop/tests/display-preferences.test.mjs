import assert from "node:assert/strict";
import test from "node:test";
import {
  connectionDisplayOptions,
  displayConfig,
  preferencesFromConfig,
} from "../src/displayPreferences.ts";

const negotiated = {
  mode: "extend",
  width: 1920,
  height: 1080,
  fps: 60,
  bitrate: 20_000_000,
  retina: true,
  auto_size: true,
  codec: "h264",
  separate_cursor: false,
};

test("changing quality preserves negotiated auto size, codec and cursor settings", () => {
  const preferences = { ...preferencesFromConfig(negotiated), quality: "detail" };
  assert.deepEqual(displayConfig("extend", preferences, negotiated), {
    ...negotiated,
    bitrate: 40_000_000,
  });
});

test("connection choices do not advertise a larger size or higher frame rate", () => {
  const options = connectionDisplayOptions(negotiated);
  assert.deepEqual(options.frameRates, [30, 60]);
  assert.equal(options.retina, true);
  for (const size of options.sizes) {
    const [w, h] = size.split("x").map(Number);
    assert.ok(w <= negotiated.width && h <= negotiated.height);
    assert.ok(w >= 640 && h >= 480);
  }
  assert.equal(options.sizes.includes("3840x2160"), false);
});

test("lowering settings does not lose the original profile during this connection", () => {
  const lowered = { ...negotiated, width: 960, height: 540, fps: 30, retina: false };
  const options = connectionDisplayOptions(negotiated, lowered);
  assert.ok(options.sizes.includes("1920x1080"));
  assert.deepEqual(options.frameRates, [30, 60]);
  assert.equal(options.retina, true);
});

test("standard-density 30 FPS connections do not offer Retina or 60/120 FPS", () => {
  const options = connectionDisplayOptions({ ...negotiated, fps: 30, retina: false });
  assert.deepEqual(options.frameRates, [30]);
  assert.equal(options.retina, false);
});

test("non-preset negotiated frame rates survive an unchanged apply", () => {
  const profile = { ...negotiated, fps: 50 };
  assert.deepEqual(displayConfig("extend", preferencesFromConfig(profile), profile), profile);
  assert.deepEqual(connectionDisplayOptions(profile).frameRates, [30, 50]);
});
