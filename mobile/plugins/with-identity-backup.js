const { AndroidConfig, withAndroidManifest, withDangerousMod } = require("expo/config-plugins");
const { mkdir, writeFile } = require("node:fs/promises");
const path = require("node:path");

// Identity, trust and credentials already live in noBackupFilesDir. Restore only
// harmless appearance preferences; peer metadata and opt-ins belong to that identity.
const rules = () => '    <include domain="sharedpref" path="appearance.xml"/>';
const legacy = `<?xml version="1.0" encoding="utf-8"?>\n<full-backup-content>\n${rules()}\n</full-backup-content>\n`;
const modern = `<?xml version="1.0" encoding="utf-8"?>\n<data-extraction-rules>\n  <cloud-backup>\n${rules()}\n  </cloud-backup>\n  <device-transfer>\n${rules()}\n  </device-transfer>\n</data-extraction-rules>\n`;

module.exports = function withIdentityBackup(config) {
  if (config.android?.allowBackup === false) {
    throw new Error("Identity backup policy requires allowBackup=true to preserve appearance only.");
  }
  config = withAndroidManifest(config, mod => {
    const application = AndroidConfig.Manifest.getMainApplicationOrThrow(mod.modResults);
    application.$["android:allowBackup"] = "true";
    application.$["android:fullBackupContent"] = "@xml/extend_identity_backup";
    application.$["android:dataExtractionRules"] = "@xml/extend_identity_extraction";
    return mod;
  });
  return withDangerousMod(config, ["android", async mod => {
    const directory = path.join(mod.modRequest.platformProjectRoot, "app/src/main/res/xml");
    await mkdir(directory, { recursive: true });
    await writeFile(path.join(directory, "extend_identity_backup.xml"), legacy);
    await writeFile(path.join(directory, "extend_identity_extraction.xml"), modern);
    return mod;
  }]);
};
