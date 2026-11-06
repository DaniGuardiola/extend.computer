const assert = require("node:assert/strict");
const { mkdtemp, readFile, readdir, rm } = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");
const xcode = require("xcode");
const plugin = require("./with-broadcast-extension");

const base = { name: "extend.computer", slug: "test", version: "1.2.3", ios: { bundleIdentifier: "computer.extend.test", appleTeamId: "TEAM123456" } };
const ids = { group: "group.computer.extend.test", extension: "computer.extend.test.broadcast", keychain: "$(AppIdentifierPrefix)computer.extend.test" };
// Any real project works as a fixture; the plugin only needs a first target to embed into.
const fixture = require.resolve("react-native-svg/apple/RNSVG.xcodeproj/project.pbxproj");
const run = (mods, type, modResults, extra = {}) => mods.ios[type]({ ...base, modResults, modRequest: { platformProjectRoot: extra.root ?? "/unused" } });

test("app gets the App Group, keychain group and the identifiers the Swift code reads", async () => {
  const config = plugin(structuredClone(base));
  const entitlements = { "keychain-access-groups": ["$(AppIdentifierPrefix)other"] };
  await run(config.mods, "entitlements", entitlements);
  await run(config.mods, "entitlements", entitlements);
  assert.deepEqual(entitlements, {
    "keychain-access-groups": ["$(AppIdentifierPrefix)other", ids.keychain],
    "com.apple.security.application-groups": [ids.group],
  });
  const info = { CFBundleName: "app" };
  await run(config.mods, "infoPlist", info);
  assert.deepEqual(info, { CFBundleName: "app", ExtendAppGroup: ids.group, ExtendBroadcastExtension: ids.extension });
  assert.throws(() => plugin({ name: "x", slug: "x" }), /bundleIdentifier/);
  const app = JSON.parse(await readFile(path.join(__dirname, "../app.json"), "utf8"));
  assert.ok(app.expo.plugins.includes("./plugins/with-broadcast-extension"));
  assert.equal(app.expo.ios.bundleIdentifier, "computer.extend.ios.development");
});

test("prebuild copies the extension sources, shared contract, Info.plist and entitlements", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "extend-broadcast-"));
  try {
    const config = plugin(structuredClone(base));
    await run(config.mods, "dangerous", {}, { root });
    const directory = path.join(root, plugin.TARGET);
    const files = (await readdir(directory)).sort();
    const sources = (await readdir(path.join(__dirname, "../targets/broadcast"))).filter(name => name.endsWith(".swift"));
    assert.deepEqual(files, [...sources, "BroadcastShared.swift", "Engine.swift", "Identity.swift",
      "ExtendBroadcast-Info.plist", "ExtendBroadcast.entitlements"].sort());
    // The extension compiles the engine facade, which must stay free of Expo.
    for (const name of ["Engine.swift", "Identity.swift", ...sources]) {
      assert.doesNotMatch(await readFile(path.join(directory, name), "utf8"), /import (ExpoModulesCore|React)/, name);
    }
    assert.ok(sources.includes("SampleHandler.swift"));
    const info = await readFile(path.join(directory, "ExtendBroadcast-Info.plist"), "utf8");
    for (const expected of ["com.apple.broadcast-services-upload", "$(PRODUCT_MODULE_NAME).SampleHandler",
      "RPBroadcastProcessModeSampleBuffer", `<string>${ids.group}</string>`, `<string>${ids.extension}</string>`,
      "<key>CFBundleShortVersionString</key>\n\t<string>1.2.3</string>"]) {
      assert.ok(info.includes(expected), expected);
    }
    const entitlements = await readFile(path.join(directory, "ExtendBroadcast.entitlements"), "utf8");
    assert.ok(entitlements.includes(`<string>${ids.group}</string>`));
    assert.ok(entitlements.includes(`<string>${ids.keychain}</string>`));
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("Xcode project gains one embedded, signed extension target", async () => {
  const project = xcode.project(fixture);
  project.parseSync();
  const app = project.getFirstTarget().uuid;
  const config = plugin(structuredClone(base));
  await run(config.mods, "xcodeproj", project, { root: path.join(__dirname, "../ios") });
  await run(config.mods, "xcodeproj", project, { root: path.join(__dirname, "../ios") }); // idempotent on an existing project
  const objects = project.hash.project.objects;
  const targets = Object.entries(objects.PBXNativeTarget).filter(([, t]) => typeof t === "object" && t.name === `"${plugin.TARGET}"`);
  assert.equal(targets.length, 1);
  const [uuid, target] = targets[0];
  assert.equal(target.productType, '"com.apple.product-type.app-extension"');
  const phases = target.buildPhases.map(phase => phase.comment);
  assert.deepEqual(phases, ["Sources", "Resources", "Frameworks"]);
  const sources = objects.PBXSourcesBuildPhase[target.buildPhases[0].value].files.map(file => file.comment.replace(/"/g, "")).sort();
  for (const name of ["SampleHandler", "BroadcastShared", "Engine", "Identity"]) assert.ok(sources.includes(`${name}.swift in Sources`), name);
  assert.ok(sources.every(name => name.endsWith(".swift in Sources")));

  const embed = Object.values(objects.PBXCopyFilesBuildPhase).filter(phase => typeof phase === "object" && phase.files.some(file => file.comment.includes("ExtendBroadcast.appex")));
  assert.equal(embed.length, 1);
  assert.equal(embed[0].dstSubfolderSpec, 13); // PlugIns
  assert.equal(embed[0].name, '"Embed Foundation Extensions"');
  assert.ok(objects.PBXNativeTarget[app].buildPhases.some(phase => phase.comment === "Embed Foundation Extensions"));
  assert.ok(objects.PBXNativeTarget[app].dependencies.length >= 1);

  const list = objects.XCConfigurationList[target.buildConfigurationList];
  const settings = list.buildConfigurations.map(({ value }) => objects.XCBuildConfiguration[value].buildSettings);
  assert.equal(settings.length, 2);
  for (const value of settings) {
    assert.equal(value.PRODUCT_BUNDLE_IDENTIFIER, `"${ids.extension}"`);
    assert.equal(value.CODE_SIGN_ENTITLEMENTS, '"ExtendBroadcast/ExtendBroadcast.entitlements"');
    assert.equal(value.INFOPLIST_FILE, '"ExtendBroadcast/ExtendBroadcast-Info.plist"');
    assert.equal(value.DEVELOPMENT_TEAM, "TEAM123456");
    assert.equal(value.APPLICATION_EXTENSION_API_ONLY, "YES");
    assert.equal(value.MARKETING_VERSION, '"1.2.3"');
    assert.equal(value.DEAD_CODE_STRIPPING, "YES");
    assert.equal(value.SWIFT_INCLUDE_PATHS, '"$(SRCROOT)/../modules/extend-display/ios/ExtendEngine.xcframework/ios-arm64/Headers"');
    for (const [sdk, slice] of [["iphoneos", "ios-arm64"], ["iphonesimulator", "ios-arm64-simulator"]]) {
      const flags = value[`"OTHER_LDFLAGS[sdk=${sdk}*]"`];
      assert.ok(flags.includes(`"$(SRCROOT)/../modules/extend-display/ios/ExtendEngine.xcframework/${slice}/libextend_ios_engine.a"`), sdk);
      assert.ok(flags.includes('"Security"'));
    }
  }
  assert.equal(project.pbxProjectSection()[project.getFirstProject().uuid].attributes.TargetAttributes[uuid].DevelopmentTeam, "TEAM123456");
  assert.doesNotThrow(() => project.writeSync());
});
