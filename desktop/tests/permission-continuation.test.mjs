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
