const assert = require("node:assert/strict");
const { test } = require("node:test");
const plugin = require("./with-standalone-debuggable");

test("standalone debug opt-in is package restricted and clears on the next normal prebuild", async () => {
  const original = process.env.EXTEND_ANDROID_STANDALONE_DEBUGGABLE;
  const manifest = { manifest: { application: [{ $: { "android:name": ".MainApplication", "android:usesCleartextTraffic": "false" } }] } };
  const config = { name: "test", slug: "test", android: { package: "computer.extend.android.development" } };
  try {
    process.env.EXTEND_ANDROID_STANDALONE_DEBUGGABLE = "1";
    const enabled = plugin({ ...config, mods: undefined });
    await enabled.mods.android.manifest({ modResults: manifest, modRequest: {} });
    assert.equal(manifest.manifest.application[0].$["android:debuggable"], "true");
    assert.equal(manifest.manifest.$["xmlns:tools"], "http://schemas.android.com/tools");
    assert.equal(manifest.manifest.application[0].$["tools:ignore"], "HardcodedDebugMode");
    manifest.manifest.application[0].$["tools:ignore"] = "UnusedAttribute,HardcodedDebugMode";
    await enabled.mods.android.manifest({ modResults: manifest, modRequest: {} });
    assert.equal(manifest.manifest.application[0].$["tools:ignore"], "UnusedAttribute,HardcodedDebugMode");
    assert.throws(() => plugin({ ...config, android: { package: "computer.extend.android" } }), /restricted/);
    process.env.EXTEND_ANDROID_STANDALONE_DEBUGGABLE = "true";
    assert.throws(() => plugin({ ...config, mods: undefined }), /0 or 1/);
    delete process.env.EXTEND_ANDROID_STANDALONE_DEBUGGABLE;
    const normal = plugin({ ...config, mods: undefined });
    await normal.mods.android.manifest({ modResults: manifest, modRequest: {} });
    assert.deepEqual(manifest.manifest.application[0].$, {
      "android:name": ".MainApplication", "android:usesCleartextTraffic": "false",
      "tools:ignore": "UnusedAttribute",
    });
  } finally {
    if (original === undefined) delete process.env.EXTEND_ANDROID_STANDALONE_DEBUGGABLE;
    else process.env.EXTEND_ANDROID_STANDALONE_DEBUGGABLE = original;
  }
});
