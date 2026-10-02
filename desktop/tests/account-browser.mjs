// UI-only isolated Tauri bridge mock; native/network coverage lives in accounts.rs and web tests.
import { chromium } from "../../web/node_modules/playwright/index.mjs";
import assert from "node:assert/strict";
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  viewport: { width: 760, height: 650 },
});
await context.addInitScript(() => {
  const account = {
    server: "https://extend.computer",
    email: null,
    device_id: null,
    devices: [],
    pending: null,
    browser_pending: false,
    error: null,
  };
  const perms = {
    listen: true,
    post: true,
    wifi: true,
    available: true,
    wifi_pending: false,
    wifi_installing: false,
    wifi_setup_failed: false,
  };
  const snapshot = {
    development: true,
    discoverable: false,
    addresses: [],
    peers: [],
    removed_peers: [],
    notification: null,
    session: null,
    receiving: false,
    code: null,
    code_seconds: 0,
    approval: null,
    error: null,
    permissions: perms,
    permission_request: null,
  };
  const calls = [];
  window.isTauri = true;
  window.__accountSmoke = { account, calls };
  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main" },
    },
    transformCallback: () => 1,
    invoke: async (command, args) => {
      if (command.startsWith("account_"))
        calls.push({ command, page: args?.page });
      if (command === "snapshot") return snapshot;
      if (command === "check_permissions") return perms;
      if (command === "local_device_info")
        return { name: "Test Mac", identity: "a".repeat(64), version: "0.1.0" };
      if (command === "account_status") return structuredClone(account);
      if (command === "account_open_page") return null;
      if (command === "account_login") {
        account.pending = { totp: true, keys: true, recovery: true };
        return structuredClone(account);
      }
      if (command === "account_verify") {
        account.pending = null;
        account.email = "test@example.invalid";
        account.device_id = "self";
        account.devices = [
          {
            id: "other",
            name: "Other Mac",
            platform: "macos",
            fingerprint: "b".repeat(64),
            online: true,
            last_seen: Math.floor(Date.now() / 1000),
          },
        ];
        return structuredClone(account);
      }
      if (command === "account_logout") {
        account.email = null;
        account.devices = [];
        return structuredClone(account);
      }
      if (command === "account_refresh") return structuredClone(account);
      return null;
    },
  };
});
const page = await context.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.goto(process.env.EXTEND_DESKTOP_TEST_URL ?? "http://localhost:1425");
await page.getByRole("button", { name: "Log in", exact: true }).click();
const dialog = page.getByRole("dialog", { name: "Log in", exact: true });
await dialog.getByRole("button", { name: /Create account on website/ }).click();
assert.equal(
  await page.evaluate(() => window.__accountSmoke.calls.at(-1).page),
  "signup",
);
await dialog
  .getByRole("textbox", { name: "Email", exact: true })
  .fill("test@example.invalid");
await dialog
  .getByLabel("Password", { exact: true })
  .fill("disposable-password");
await dialog.getByRole("button", { name: "Log in", exact: true }).click();
await dialog
  .getByRole("textbox", { name: "Authenticator or recovery code", exact: true })
  .fill("123456");
await dialog.getByRole("button", { name: "Verify code", exact: true }).click();
await page.getByText("test@example.invalid", { exact: true }).waitFor();
await page.screenshot({ path: "/tmp/extend-desktop-account.png" });
await page.getByRole("button", { name: "Close", exact: true }).click();
await page
  .getByRole("heading", { name: "Account devices", exact: true })
  .waitFor();
await page.getByText("Other Mac", { exact: true }).waitFor();
await page.getByRole("button", { name: "Account", exact: true }).click();
await page.getByRole("button", { name: /Manage account & security/ }).click();
assert.equal(
  await page.evaluate(() => window.__accountSmoke.calls.at(-1).page),
  "account",
);
await page.getByRole("button", { name: "Sign out", exact: true }).click();
await page.getByRole("textbox", { name: "Email", exact: true }).waitFor();
assert.deepEqual(errors, []);
assert.equal(
  await page.evaluate(() =>
    Object.keys(localStorage).some((k) => /token|session|password/.test(k)),
  ),
  false,
);
console.log(
  JSON.stringify({
    desktopUI: true,
    passwordAndFactor: true,
    signupOpensWebsite: true,
    accountDevices: true,
    accountSettingsWebsite: true,
    logout: true,
    noBrowserErrors: true,
  }),
);
await context.close();
await browser.close();
