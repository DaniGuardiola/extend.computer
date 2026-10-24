import assert from "node:assert/strict";
import { test } from "node:test";
import { displayEndMessage } from "./end-message.ts";

test("terminal source stop has no empty native error detail", () => {
  for (const message of [undefined, "", "  ", "Display ended: ", "Display ended", "Display session ended.", "Connection ended."]) {
    assert.equal(displayEndMessage(message), undefined);
  }
});

test("real failure detail survives generic terminal wording", () => {
  for (const message of [
    "Display ended: Hardware decoder isn't keeping up",
    "Display ended: Device access expired",
    "Display ended: USB connection changed",
    "Connection ended: timed out",
    "Couldn't connect to your display.",
  ]) {
    assert.equal(displayEndMessage(message), message);
  }
});
