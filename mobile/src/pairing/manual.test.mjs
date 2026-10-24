import assert from "node:assert/strict";
import { test } from "node:test";
import { ManualPairingAttempt } from "./manual.ts";

function deferred() { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }
function fixture() {
  let next = 0;
  const stopped = [], starts = new Map(), reads = [];
  const bridge = {
    reserveManualPairing: () => String(++next),
    startManualPairing: id => { const request = deferred(); starts.set(id, request); return request.promise; },
    getManualPairing: id => { const request = deferred(); reads.push({ id, ...request }); return request.promise; },
    stopManualPairing: id => stopped.push(id),
  };
  return { attempt: new ManualPairingAttempt(bridge), stopped, starts, reads };
}
const state = (id, phase, extra = {}) => JSON.stringify({ id, state: phase, ...extra });

test("cancel owns an id before async startup; stale success cannot affect a replacement", async () => {
  const f = fixture();
  const old = f.attempt.start("old", "code", "old");
  assert.equal(f.attempt.id, "1");
  f.attempt.stop();
  const current = f.attempt.start("new", "code", "new");
  f.starts.get("1").resolve(state("1", "paired", { peer: "a".repeat(64) }));
  assert.equal(await old, null);
  assert.equal(f.attempt.id, "2");
  assert.deepEqual(f.stopped, ["1"]);
  f.starts.get("2").resolve(state("2", "connecting"));
  assert.equal((await current).id, "2");
  f.attempt.stop();
});
test("late failure and state read after leave do not stop or report against a replacement", async () => {
  const f = fixture();
  const old = f.attempt.start("old", "code", "old");
  const read = f.attempt.read();
  f.attempt.stop();
  const current = f.attempt.start("new", "code", "new");
  f.starts.get("1").reject(new Error("stale native start"));
  f.reads[0].reject(new Error("old operation removed"));
  assert.equal(await old, null); assert.equal(await read, null);
  assert.equal(f.attempt.id, "2");
  assert.deepEqual(f.stopped, ["1"]);
  f.starts.get("2").resolve(state("2", "waiting")); await current; f.attempt.stop();
});
test("polling deduplicates pending native IO and a terminal pair closes only its own reservation", async () => {
  const f = fixture();
  const started = f.attempt.start("host", "code", "name");
  f.starts.get("1").resolve(state("1", "waiting")); await started;
  const read = f.attempt.read();
  assert.equal(await f.attempt.read(), null); assert.equal(f.reads.length, 1);
  f.reads[0].resolve(state("1", "paired", { peer: "b".repeat(64) }));
  assert.equal((await read).state, "paired");
  assert.equal(f.attempt.id, undefined); assert.deepEqual(f.stopped, ["1"]);
  f.attempt.stop(); assert.deepEqual(f.stopped, ["1"]);
});
test("wrong operation and malformed paired identity fail closed without accepting the result", async () => {
  for (const raw of [state("other", "paired", { peer: "c".repeat(64) }), state("1", "paired", { peer: "wrong" }), state("1", "unknown")]) {
    const f = fixture(), started = f.attempt.start("host", "code", "name");
    f.starts.get("1").resolve(raw);
    await assert.rejects(started, /operation changed/);
    assert.equal(f.attempt.id, undefined); assert.deepEqual(f.stopped, ["1"]);
  }
});
test("duplicate start cannot reserve a second native operation; expiry retires the old one", async () => {
  const f = fixture(), started = f.attempt.start("host", "code", "name");
  await assert.rejects(f.attempt.start("host", "code", "name"), /already in progress/);
  assert.equal(f.starts.size, 1);
  f.starts.get("1").resolve(state("1", "expired"));
  assert.equal((await started).state, "expired"); assert.equal(f.attempt.id, undefined);
  assert.deepEqual(f.stopped, ["1"]);
});
