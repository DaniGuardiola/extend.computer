const { withDangerousMod, withEntitlementsPlist, withInfoPlist, withXcodeProject } = require("expo/config-plugins");
const { copyFile, mkdir, readdir, writeFile } = require("node:fs/promises");
const path = require("node:path");

// Adds the ReplayKit broadcast upload extension that sends the iPhone screen
// (Android: ScreenSharingService). Sources live in targets/broadcast plus the
// app/extension contract in modules/extend-display/ios/Sharing/BroadcastShared.swift;
// prebuild copies them into ios/ExtendBroadcast and wires a target that the app
// embeds. App and extension share an App Group (requests/state) and a keychain
// access group (engine identity seed).

const TARGET = "ExtendBroadcast";
const SOURCES = path.join(__dirname, "../targets/broadcast");
const SHARED = path.join(__dirname, "../modules/extend-display/ios/Sharing/BroadcastShared.swift");
const DEPLOYMENT_TARGET = "16.4";

function names(config) {
  const bundle = config.ios?.bundleIdentifier;
  if (!bundle) throw new Error("Broadcast extension requires expo.ios.bundleIdentifier.");
  return {
    bundle,
    extension: `${bundle}.broadcast`,
    group: `group.${bundle}`,
    keychain: `$(AppIdentifierPrefix)${bundle}`,
  };
}

const plist = body => `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
${body}
</dict>
</plist>
`;
const escape = value => String(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const entry = (key, value) => `\t<key>${key}</key>\n\t<string>${escape(value)}</string>`;

function infoPlist(config, ids) {
  return plist([
    entry("CFBundleDevelopmentRegion", "$(DEVELOPMENT_LANGUAGE)"),
    entry("CFBundleDisplayName", config.name ?? TARGET),
    entry("CFBundleExecutable", "$(EXECUTABLE_NAME)"),
    entry("CFBundleIdentifier", "$(PRODUCT_BUNDLE_IDENTIFIER)"),
    entry("CFBundleInfoDictionaryVersion", "6.0"),
    entry("CFBundleName", "$(PRODUCT_NAME)"),
    entry("CFBundlePackageType", "$(PRODUCT_BUNDLE_PACKAGE_TYPE)"),
    // Must match the containing app or App Store validation rejects the build.
    entry("CFBundleShortVersionString", config.version ?? "1.0.0"),
    entry("CFBundleVersion", config.ios?.buildNumber ?? "1"),
    entry("ExtendAppGroup", ids.group),
    entry("ExtendBroadcastExtension", ids.extension),
    "\t<key>NSExtension</key>\n\t<dict>",
    "\t\t<key>NSExtensionPointIdentifier</key>\n\t\t<string>com.apple.broadcast-services-upload</string>",
    "\t\t<key>NSExtensionPrincipalClass</key>\n\t\t<string>$(PRODUCT_MODULE_NAME).SampleHandler</string>",
    "\t\t<key>RPBroadcastProcessMode</key>\n\t\t<string>RPBroadcastProcessModeSampleBuffer</string>",
    "\t</dict>",
  ].join("\n"));
}

function entitlementsPlist(ids) {
  return plist([
    "\t<key>com.apple.security.application-groups</key>",
    `\t<array>\n\t\t<string>${ids.group}</string>\n\t</array>`,
    "\t<key>keychain-access-groups</key>",
    `\t<array>\n\t\t<string>${ids.keychain}</string>\n\t</array>`,
  ].join("\n"));
}

async function sourceFiles() {
  const own = (await readdir(SOURCES)).filter(name => name.endsWith(".swift")).sort();
  return [...own, path.basename(SHARED)];
}

const withAppEntitlements = config => withEntitlementsPlist(config, mod => {
  const ids = names(mod);
  const add = (key, value) => {
    const list = Array.isArray(mod.modResults[key]) ? mod.modResults[key] : [];
    if (!list.includes(value)) list.push(value);
    mod.modResults[key] = list;
  };
  add("com.apple.security.application-groups", ids.group);
  add("keychain-access-groups", ids.keychain);
  return mod;
});

const withAppInfo = config => withInfoPlist(config, mod => {
  const ids = names(mod);
  mod.modResults.ExtendAppGroup = ids.group;
  mod.modResults.ExtendBroadcastExtension = ids.extension;
  return mod;
});

const withExtensionFiles = config => withDangerousMod(config, ["ios", async mod => {
  const ids = names(mod);
  const directory = path.join(mod.modRequest.platformProjectRoot, TARGET);
  await mkdir(directory, { recursive: true });
  for (const name of (await readdir(SOURCES)).filter(name => name.endsWith(".swift"))) {
    await copyFile(path.join(SOURCES, name), path.join(directory, name));
  }
  await copyFile(SHARED, path.join(directory, path.basename(SHARED)));
  await writeFile(path.join(directory, `${TARGET}-Info.plist`), infoPlist(mod, ids));
  await writeFile(path.join(directory, `${TARGET}.entitlements`), entitlementsPlist(ids));
  return mod;
}]);

function configureTarget(project, target, config, ids) {
  const quote = value => `"${value}"`;
  const configurations = project.pbxXCBuildConfigurationSection();
  const list = project.pbxXCConfigurationList()[target.pbxNativeTarget.buildConfigurationList];
  for (const { value } of list.buildConfigurations) {
    const configuration = configurations[value];
    const debug = configuration.name === "Debug";
    Object.assign(configuration.buildSettings, {
      APPLICATION_EXTENSION_API_ONLY: "YES",
      CLANG_ENABLE_MODULES: "YES",
      CODE_SIGN_ENTITLEMENTS: quote(`${TARGET}/${TARGET}.entitlements`),
      CODE_SIGN_STYLE: "Automatic",
      CURRENT_PROJECT_VERSION: quote(config.ios?.buildNumber ?? "1"),
      ...(config.ios?.appleTeamId ? { DEVELOPMENT_TEAM: config.ios.appleTeamId } : {}),
      GENERATE_INFOPLIST_FILE: "NO",
      INFOPLIST_FILE: quote(`${TARGET}/${TARGET}-Info.plist`),
      IPHONEOS_DEPLOYMENT_TARGET: DEPLOYMENT_TARGET,
      MARKETING_VERSION: quote(config.version ?? "1.0.0"),
      PRODUCT_BUNDLE_IDENTIFIER: quote(ids.extension),
      PRODUCT_NAME: quote(TARGET),
      SKIP_INSTALL: "YES",
      SWIFT_OPTIMIZATION_LEVEL: quote(debug ? "-Onone" : "-O"),
      SWIFT_VERSION: "5.0",
      TARGETED_DEVICE_FAMILY: quote("1,2"),
    });
  }
}

const withExtensionTarget = config => withXcodeProject(config, async mod => {
  const project = mod.modResults;
  const ids = names(mod);
  // Re-run on an existing project (`prebuild` without --clean). addTarget stores the name quoted.
  if (project.pbxTargetByName(TARGET) || project.pbxTargetByName(`"${TARGET}"`)) return mod;

  const sources = await sourceFiles();
  const files = [...sources, `${TARGET}-Info.plist`, `${TARGET}.entitlements`];
  const group = project.addPbxGroup(files, TARGET, TARGET);
  const mainGroup = project.getFirstProject().firstProject.mainGroup;
  project.addToPbxGroup(group.uuid, mainGroup);

  // Older templates lack these sections; addTarget assumes they exist.
  const objects = project.hash.project.objects;
  objects.PBXTargetDependency ??= {};
  objects.PBXContainerItemProxy ??= {};

  const target = project.addTarget(TARGET, "app_extension", TARGET, ids.extension);
  project.addBuildPhase(sources, "PBXSourcesBuildPhase", "Sources", target.uuid);
  project.addBuildPhase([], "PBXResourcesBuildPhase", "Resources", target.uuid);
  project.addBuildPhase([], "PBXFrameworksBuildPhase", "Frameworks", target.uuid);
  configureTarget(project, target, mod, ids);

  // addTarget names the app's embed phase "Copy Files"; give it Xcode's name.
  const copyPhases = objects.PBXCopyFilesBuildPhase ?? {};
  for (const key of Object.keys(copyPhases)) {
    const phase = copyPhases[key];
    if (typeof phase === "object" && phase.files?.some(file => String(file.comment).includes(`${TARGET}.appex`))) {
      phase.name = '"Embed Foundation Extensions"';
      copyPhases[`${key}_comment`] = "Embed Foundation Extensions";
      for (const native of Object.values(objects.PBXNativeTarget)) {
        for (const reference of native.buildPhases ?? []) if (reference.value === key) reference.comment = "Embed Foundation Extensions";
      }
    }
  }

  if (config.ios?.appleTeamId) {
    project.addTargetAttribute("DevelopmentTeam", config.ios.appleTeamId, target);
    project.addTargetAttribute("ProvisioningStyle", "Automatic", target);
  }
  return mod;
});

module.exports = function withBroadcastExtension(config) {
  names(config);
  config = withAppEntitlements(config);
  config = withAppInfo(config);
  config = withExtensionFiles(config);
  return withExtensionTarget(config);
};
module.exports.TARGET = TARGET;
module.exports.names = names;
