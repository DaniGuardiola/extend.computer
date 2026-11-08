import assert from "node:assert/strict";
import test from "node:test";
import { hasDesktopToView } from "../src/remoteDesktop.ts";

const computerOffers = [
  "display-source-mirror",
  "display-viewer",
  "input-receive",
  "input-send",
];

test("phones and tablets have no desktop to view", () => {
  for (const kind of ["phone", "tablet"])
    for (const platform of ["android", "ios", "ipados", "macos"])
      assert.equal(
        hasDesktopToView({ device_info: { platform, kind } }, platform),
        false,
        `${platform} ${kind}`,
      );
});

test("computers keep the tile, whatever their platform", () => {
  for (const kind of ["desktop", "laptop"])
    for (const platform of ["macos", "windows", "linux"])
      assert.equal(
        hasDesktopToView(
          { device_info: { platform, kind }, offers: computerOffers },
          platform,
        ),
        true,
        `${platform} ${kind}`,
      );
});

test("without reported hardware, the platform decides", () => {
  // The card falls back to the account directory's platform.
  for (const platform of ["android", "ios", "iPadOS"])
    assert.equal(hasDesktopToView({}, platform), false, platform);
  for (const platform of ["macos", "windows", "linux"])
    assert.equal(hasDesktopToView({}, platform), true, platform);
  // A device of unknown type keeps the tile.
  assert.equal(hasDesktopToView({}, undefined), true);
  assert.equal(
    hasDesktopToView({ device_info: { platform: "beos", kind: "unknown" } }),
    true,
  );
});

test("a device that said it can't share its screen has none to view", () => {
  assert.equal(
    hasDesktopToView(
      {
        device_info: { platform: "macos", kind: "laptop" },
        offers: ["display-viewer", "input-receive"],
      },
      "macos",
    ),
    false,
  );
  // Older builds don't list their roles; they keep the tile.
  assert.equal(
    hasDesktopToView(
      { device_info: { platform: "macos", kind: "laptop" }, offers: null },
      "macos",
    ),
    true,
  );
});
