const assert = require("node:assert/strict");
const { mkdtemp, readFile, rm, writeFile } = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");
const { parseStringPromise } = require("xml2js");
const plugin = require("./with-identity-backup");

const protectedPrefs = ["account-config.xml", "engine-labels.xml", "engine-local.xml", "engine-peers.xml", "engine-screen-sharing.xml"];
function verifyPolicy(policy) {
  assert.deepEqual(policy.include, [{ $: { domain: "sharedpref", path: "appearance.xml" } }]);
  assert.equal(policy.exclude, undefined, "Excludes outside the include allowlist fail Android release lint");
  // Android includes switch the default to an allowlist: no other files/databases
  // or future preferences are silently restored without their corresponding identity.
  for (const name of [...protectedPrefs, "future-peer-store.xml"]) {
    assert.equal(policy.include.some(entry => entry.$.path === name || entry.$.path === "."), false);
  }
}

test("legacy/cloud/device transfer allow only appearance; repeated generation preserves unrelated resources", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "extend-backup-"));
  try {
    const config = plugin({ name: "test", slug: "test", android: { allowBackup: true } });
    const request = { modRequest: { platformProjectRoot: directory }, modResults: {} };
    await config.mods.android.dangerous(request);
    const xml = path.join(directory, "app/src/main/res/xml");
    await writeFile(path.join(xml, "unrelated.xml"), "preserve");
    const legacyPath = path.join(xml, "extend_identity_backup.xml");
    const modernPath = path.join(xml, "extend_identity_extraction.xml");
    const before = [await readFile(legacyPath,"utf8"), await readFile(modernPath,"utf8")];
    await config.mods.android.dangerous(request);
    assert.deepEqual([await readFile(legacyPath,"utf8"), await readFile(modernPath,"utf8")], before);
    assert.equal(await readFile(path.join(xml,"unrelated.xml"),"utf8"),"preserve");
    verifyPolicy((await parseStringPromise(before[0]))["full-backup-content"]);
    const modern = (await parseStringPromise(before[1]))["data-extraction-rules"];
    assert.deepEqual(Object.keys(modern).sort(), ["cloud-backup", "device-transfer"]);
    verifyPolicy(modern["cloud-backup"][0]);
    verifyPolicy(modern["device-transfer"][0]);
  } finally { await rm(directory, {recursive:true,force:true}); }
});

test("manifest policy preserves security/signing/package fields and rejects contradictory configuration", async () => {
  assert.throws(() => plugin({android:{allowBackup:false}}), /allowBackup=true/);
  const config = plugin({name:"test",slug:"test"});
  const attrs = {"android:name":".MainApplication","android:debuggable":"false","android:networkSecurityConfig":"@xml/existing"};
  const mod = {modResults:{manifest:{application:[{$:{...attrs}}]}},modRequest:{}};
  await config.mods.android.manifest(mod);
  await config.mods.android.manifest(mod);
  assert.deepEqual(mod.modResults.manifest.application[0].$, {...attrs,
    "android:allowBackup":"true","android:fullBackupContent":"@xml/extend_identity_backup",
    "android:dataExtractionRules":"@xml/extend_identity_extraction"});
  const app = JSON.parse(await readFile(path.join(__dirname,"../app.json"),"utf8"));
  assert.ok(app.expo.plugins.includes("./plugins/with-identity-backup"));
  assert.equal(app.expo.android.package,"computer.extend.android.development");
});

test("all native preference stores remain explicitly reviewed", async () => {
  const { readdir } = require("node:fs/promises");
  const native = path.join(__dirname,"../../native/android/src/computer/extend/android");
  const stores = new Set();
  for (const name of await readdir(native)) {
    if (!name.endsWith(".java")) continue;
    const source = await readFile(path.join(native,name),"utf8");
    for (const match of source.matchAll(/getSharedPreferences\("([^"]+)"/g)) stores.add(match[1]+".xml");
  }
  assert.deepEqual([...stores].sort(), protectedPrefs);
});
