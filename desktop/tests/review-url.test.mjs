import assert from "node:assert/strict";
import test from "node:test";
import { scenarios } from "../src/review/scenarios.ts";
import { readReviewURL, writeReviewURL } from "../src/review/url-state.ts";

const base = "http://127.0.0.1:1421/review.html";
const defaults = {
  state: "empty",
  view: "fit",
  theme: "light",
  sidebar: false,
  filter: "",
};

test("every preview layout and theme survives a copied deep link", () => {
  for (const view of ["fit", "desktop", "compact", "wide", "narrow", "compare"])
    for (const theme of ["light", "dark", "system"]) {
      const selected = {
        state: "one",
        view,
        theme,
        sidebar: true,
        filter: "USB",
      };
      const link = writeReviewURL(base, selected).href;
      assert.deepEqual(readReviewURL(link), selected, `${view}/${theme}`);
    }
});

test("filters preserve spaces, reserved URL characters, and Unicode", () => {
  const filter = "  usb & account?x=1#note + 100% / 📱  ";
  const link = writeReviewURL(base, { ...defaults, sidebar: true, filter });
  assert.equal(link.hash, "");
  assert.equal(readReviewURL(link.href).filter, filter);
});

test("closing the catalogue retains its filter across reload", () => {
  const opened = writeReviewURL(base, {
    ...defaults,
    sidebar: true,
    filter: "permissions",
  });
  const closed = writeReviewURL(opened, {
    ...readReviewURL(opened),
    sidebar: false,
  });
  assert.deepEqual(readReviewURL(closed.href), {
    ...defaults,
    filter: "permissions",
  });
  const cleared = writeReviewURL(closed, {
    ...readReviewURL(closed),
    filter: "",
  });
  assert.equal(cleared.searchParams.has("filter"), false);
  assert.deepEqual(readReviewURL(cleared.href), defaults);
});

test("missing and invalid controls fall back to a valid first screen", () => {
  assert.deepEqual(readReviewURL(base), defaults);
  assert.deepEqual(
    readReviewURL(
      `${base}?state=missing-screen&view=maximized&theme=auto&sidebar=yes`,
    ),
    defaults,
  );
  assert.deepEqual(
    readReviewURL(`${base}?state=&view=&theme=&sidebar=&filter=`),
    defaults,
  );
});

test("writing review controls preserves unrelated parameters and the fragment", () => {
  const input = new URL(`${base}?debug=1&tag=first&tag=second#comment-2`);
  const original = input.href;
  const result = writeReviewURL(input, { ...defaults, view: "narrow" });
  assert.equal(result.searchParams.get("debug"), "1");
  assert.deepEqual(result.searchParams.getAll("tag"), ["first", "second"]);
  assert.equal(result.hash, "#comment-2");
  assert.equal(input.href, original, "The caller's URL remains unchanged");
  assert.equal(readReviewURL(result).view, "narrow");
});

test("fixture deep links retain their identity when the catalogue is reordered", () => {
  const selected = scenarios[1];
  const link = writeReviewURL(base, { ...defaults, state: selected.id }).href;
  const original = [...scenarios];
  try {
    scenarios.reverse();
    assert.equal(readReviewURL(link).state, selected.id);
    assert.equal(
      readReviewURL(writeReviewURL(link, readReviewURL(link))).state,
      selected.id,
    );
  } finally {
    scenarios.splice(0, scenarios.length, ...original);
  }
});
