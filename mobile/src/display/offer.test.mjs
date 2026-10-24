import assert from "node:assert/strict";
import { test } from "node:test";
import { reconnectOffer } from "./offer.ts";

const fresh = { id: "new", available: true, host: "Paired Mac", mode: "extend" };

test("terminal reconnect accepts only the same fresh offer in both native snapshots", () => {
  assert.equal(reconnectOffer("old", { state: "offered", id: "new" }, fresh), fresh);
  assert.equal(reconnectOffer(undefined, { state: "offered", id: "new" }, fresh), fresh);
  for (const state of ["streaming", "connecting", "configuring", "closed", "failed"]) {
    assert.equal(reconnectOffer("old", { state, id: "new" }, fresh), null);
  }
  assert.equal(reconnectOffer("new", { state: "offered", id: "new" }, fresh), null);
  assert.equal(reconnectOffer("old", { state: "offered", id: "new" }, { ...fresh, id: "replacement" }), null);
  assert.equal(reconnectOffer("old", { state: "offered", id: "new" }, { ...fresh, available: false }), null);
  assert.equal(reconnectOffer("old", { state: "offered", id: "" }, { ...fresh, id: "" }), null);
  assert.equal(reconnectOffer("old", { state: "offered" }, fresh), null);
});
