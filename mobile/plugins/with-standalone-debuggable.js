const { AndroidConfig, withAndroidManifest } = require("expo/config-plugins");

// Manifest-only debugging preserves release JS bundling and ReactBuildConfig.DEBUG=false.
// Never change the release build type, package, signing key, or developer-support host.
module.exports = function withStandaloneDebuggable(config) {
  const flag = process.env.EXTEND_ANDROID_STANDALONE_DEBUGGABLE;
  if (flag !== undefined && flag !== "0" && flag !== "1") {
    throw new Error("EXTEND_ANDROID_STANDALONE_DEBUGGABLE must be 0 or 1.");
  }
  const enabled = flag === "1";
  if (enabled && config.android?.package !== "computer.extend.android.development") {
    throw new Error("Standalone debugging is restricted to computer.extend.android.development.");
  }
  return withAndroidManifest(config, (mod) => {
    const application = AndroidConfig.Manifest.getMainApplicationOrThrow(mod.modResults);
    const ignored = (application.$["tools:ignore"] || "").split(",").map((item) => item.trim()).filter((item) => item && item !== "HardcodedDebugMode");
    if (enabled) {
      mod.modResults.manifest.$ = mod.modResults.manifest.$ || {};
      mod.modResults.manifest.$["xmlns:tools"] = "http://schemas.android.com/tools";
      ignored.push("HardcodedDebugMode");
    }
    if (ignored.length) application.$["tools:ignore"] = ignored.join(",");
    else delete application.$["tools:ignore"];
    if (enabled) application.$["android:debuggable"] = "true";
    else delete application.$["android:debuggable"];
    return mod;
  });
};
