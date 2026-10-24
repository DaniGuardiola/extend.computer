const assert = require("node:assert/strict");
const { readFile } = require("node:fs/promises");
const path = require("node:path");
const { test } = require("node:test");
const { parseStringPromise } = require("xml2js");
const plugin = require("./with-account-network");

test("prebuild retains the account policy without weakening unrelated manifest fields", async () => {
  const config = plugin({ name: "test", slug: "test" });
  const manifest = { manifest: { application: [{ $: { "android:name": ".MainApplication", "android:usesCleartextTraffic": "false" } }] } };
  const once = await config.mods.android.manifest({ modResults: manifest, modRequest: {} });
  const twice = await config.mods.android.manifest(once);
  assert.deepEqual(twice.modResults, manifest);
  assert.deepEqual(manifest.manifest.application[0].$, {
    "android:name": ".MainApplication",
    "android:usesCleartextTraffic": "false",
    "android:networkSecurityConfig": "@xml/extend_account_network_security",
  });
  const app = JSON.parse(await readFile(path.join(__dirname, "../app.json"), "utf8"));
  assert.ok(app.expo.plugins.includes("./plugins/with-account-network"));
});

test("HTTP policy permits exactly AccountProtocol loopback hosts and system TLS trust", async () => {
  const resource = path.join(__dirname, "../modules/extend-display/android/src/main/res/xml/extend_account_network_security.xml");
  const document = await parseStringPromise(await readFile(resource, "utf8"));
  const policy = document["network-security-config"];
  assert.deepEqual(Object.keys(policy).sort(), ["base-config", "domain-config"]);
  assert.equal(policy["base-config"].length, 1);
  assert.equal(policy["base-config"][0].$.cleartextTrafficPermitted, "false");
  assert.deepEqual(policy["base-config"][0]["trust-anchors"], [{ certificates: [{ $: { src: "system" } }] }]);
  assert.equal(policy["domain-config"].length, 1);
  const exception = policy["domain-config"][0];
  assert.equal(exception.$.cleartextTrafficPermitted, "true");
  assert.deepEqual(Object.keys(exception).sort(), ["$", "domain"]);
  const hosts = exception.domain.map((domain) => {
    assert.deepEqual(domain.$, { includeSubdomains: "false" });
    return domain._;
  });
  // URI/URL APIs can retain IPv6 brackets; both spellings name the same loopback.
  assert.deepEqual(hosts.sort(), ["127.0.0.1", "::1", "[::1]", "localhost"]);
  for (const host of ["127.0.0.2", "10.0.2.2", "192.168.1.2", "example.com", "localhost.example.com", "sub.localhost", "0.0.0.0", "::"]) {
    assert.equal(hosts.includes(host), false, `Unexpected HTTP exception: ${host}`);
  }
});
