import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DeviceRequests,
  currentStatus,
  STATUS_TTL_MS,
} from "./availability.ts";
import { validSymbols, verificationAlphabet } from "../pairing/symbols.ts";
import { readFileSync } from "node:fs";

// A UI deadline must not permit a second native request behind a stalled socket.
test("timeout retains the native request slot until actual completion", async () => {
  const requests = new DeviceRequests();
  let release;
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  assert.deepEqual(await requests.run(() => pending, 2), { kind: "timeout" });
  let entered = false;
  assert.deepEqual(
    await requests.run(async () => {
      entered = true;
    }, 2),
    { kind: "busy" },
  );
  assert.equal(entered, false);
  release("old reply");
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(await requests.run(async () => "fresh reply"), {
    kind: "done",
    value: "fresh reply",
  });
});
test("failed requests release the slot without hiding failure", async () => {
  const requests = new DeviceRequests();
  await assert.rejects(
    requests.run(async () => {
      throw new Error("native failed");
    }),
    /native failed/,
  );
  assert.equal((await requests.run(async () => true)).kind, "done");
});
test("expired authenticated status cannot keep actions online", () => {
  const online = { state: "online", checkedAt: 1000 };
  assert.equal(currentStatus(online, 1000 + STATUS_TTL_MS - 1).state, "online");
  assert.equal(currentStatus(online, 1000 + STATUS_TTL_MS).state, "checking");
  assert.equal(currentStatus(undefined, 1000).state, "checking");
});
test("verification rejects malformed and out-of-range symbol sequences", () => {
  for (const symbols of [
    null,
    [],
    [0, 1, 2, 3, 4, 5, 6],
    [0, 1, 2, 3, 4, 5, 6, 64],
    [0, 1, 2, 3, 4, 5, 6, -1],
    [0, 1, 2, 3, 4, 5, 6, 1.5],
  ])
    assert.equal(validSymbols(symbols), false);
  assert.equal(validSymbols([0, 1, 2, 3, 4, 5, 6, 63]), true);
});
test("Android uses the exact desktop verification alphabet and order", () => {
  const desktop = readFileSync(
    new URL("../../../desktop/src/VerificationSymbols.tsx", import.meta.url),
    "utf8",
  );
  const alphabet = JSON.parse(
    desktop
      .slice(desktop.indexOf("["), desktop.indexOf("] as const") + 1)
      .replace(/,\s*]/g, "]"),
  );
  assert.deepEqual(verificationAlphabet, alphabet);
  assert.equal(alphabet.length, 64);
});
