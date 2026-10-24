const { AndroidConfig, withAndroidManifest } = require("expo/config-plugins");

// The resource belongs to the local display module, so prebuild cannot discard it.
module.exports = function withAccountNetwork(config) {
  return withAndroidManifest(config, (mod) => {
    const application = AndroidConfig.Manifest.getMainApplicationOrThrow(mod.modResults);
    application.$["android:networkSecurityConfig"] = "@xml/extend_account_network_security";
    return mod;
  });
};
