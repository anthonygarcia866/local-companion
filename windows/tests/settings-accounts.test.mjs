// Settings → Claude Code & Agents → Accounts (accounts.ts; owner 2026-11-15).
import { test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { accountsSection, signedInText } = await import("../src/settings/accounts.ts");
const { sent, replies } = await import("./tauri.mjs");

const tick = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0)); };
const role = (el, tag, r) => el.find(tag).find((x) => x.getAttribute("data-role") === r);
const roleText = (el, r) => el.find("div").concat(el.find("span")).filter((d) => d.getAttribute("data-role") === r).map((d) => d.textContent);
const acct = (name, configDir, signedIn, client) => ({ name, configDir, signedIn, client });

test("accounts: each with its folder, signed in or needs login, and the client acting for it", async () => {
  replies.set("accounts_status", [acct("Claude", "C:\Users\someone\.claude", true, "account-1"), acct("Claude2", "C:\Users\someone\.claude2", false, "account-2")]);
  const el = accountsSection();
  await tick();
  assert.deepEqual(roleText(el, "account-state"), ["Signed in", "Needs login"]);
  const text = roleText(el, "account").join(" | ");
  assert.match(text, /Claude2.*\.claude2.*account-2/);
  assert.equal(signedInText(acct("x", "", true, "")), "Signed in");
});

test("accounts: Add and Remove go to Rust", async () => {
  replies.set("accounts_status", [acct("Claude", "C:\Users\someone\.claude", true, "account-1")]);
  const el = accountsSection();
  await tick();
  role(el, "input", "account-name").value = "Claude3";
  role(el, "input", "account-folder").value = "C:\Users\someone\.claude3";
  replies.set("accounts_add", [acct("Claude", "C:\Users\someone\.claude", true, "account-1"), acct("Claude3", "C:\Users\someone\.claude3", false, "account-2")]);
  role(el, "button", "account-add").fire("click");
  await tick();
  assert.deepEqual(sent("accounts_add").at(-1), { name: "Claude3", folder: "C:\Users\someone\.claude3" });
  assert.equal(roleText(el, "account").length, 2);
  replies.set("accounts_remove", [acct("Claude", "C:\Users\someone\.claude", true, "account-1")]);
  el.find("button").filter((b) => b.getAttribute("data-role") === "account-remove")[1].fire("click");
  await tick();
  assert.deepEqual(sent("accounts_remove").at(-1), { name: "Claude3" });
  assert.equal(roleText(el, "account").length, 1);
});
