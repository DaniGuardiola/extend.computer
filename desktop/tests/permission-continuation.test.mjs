import assert from "node:assert/strict";
import test from "node:test";
import { PermissionContinuation } from "../src/PermissionContinuation.ts";

const allowed = { available: true, post: true, listen: true, wifi: true };
const share = { kind: "share", peer: "peer-a", device: { name: "Other Mac", address: "192.0.2.1:48177", edge: "left" } };

test("waits for every permission and consumes the original share only once", () => {
  const pending = new PermissionContinuation();
  pending.waitFor(share);
  assert.equal(pending.takeReady({ ...allowed, wifi: false }), null);
  assert.equal(pending.takeReady({ ...allowed, listen: false }), null);
  assert.equal(pending.takeReady({ ...allowed, post: false }), null);
  assert.equal(pending.takeReady({ ...allowed, available: false }), null);
  assert.deepEqual(pending.takeReady(allowed), share);
  assert.equal(pending.takeReady(allowed), null);
});

test("cancelled action never resumes after a later grant", () => {
  const pending = new PermissionContinuation();
  pending.waitFor(share);
  pending.cancel();
  assert.equal(pending.takeReady(allowed), null);
});

test("receiving resumes without requiring input capture", () => {
  const pending = new PermissionContinuation();
  pending.waitFor({ kind: "receive" });
  assert.deepEqual(pending.takeReady({ ...allowed, listen: false }), { kind: "receive" });
});

test("new intent replaces the previous target", () => {
  const pending = new PermissionContinuation();
  pending.waitFor(share);
  pending.waitFor({ ...share, peer: "peer-b" });
  assert.equal(pending.takeReady(allowed).peer, "peer-b");
  assert.equal(pending.takeReady(allowed), null);
});

test("USB continuation still requires native input access and its current target", () => {
  const pending = new PermissionContinuation();
  pending.waitFor(share);
  const cable = (action) => action.kind === "share" && action.peer === "peer-a" ? "usb" : "local";
  const noWifi = { ...allowed, wifi: false };
  for (const key of ["available", "post", "listen"])
    assert.equal(pending.takeReady({ ...noWifi, [key]: false }, cable), null);
  assert.equal(pending.takeReady(noWifi, () => "local"), null);
  assert.deepEqual(pending.takeReady(noWifi, cable), share);
  assert.equal(pending.takeReady(noWifi, cable), null);
  pending.waitFor({ ...share, peer: "peer-b" });
  assert.equal(pending.takeReady(noWifi, cable), null);
});

test("USB receiving needs accessibility but no capture or Wi-Fi broker", () => {
  const pending = new PermissionContinuation();
  pending.waitFor({ kind: "receive" });
  const receiving = { ...allowed, listen: false, wifi: false };
  assert.equal(pending.takeReady({ ...receiving, post: false }, () => "usb"), null);
  assert.equal(pending.takeReady(receiving, () => "local"), null);
  assert.deepEqual(pending.takeReady(receiving, () => "usb"), { kind: "receive" });
});
