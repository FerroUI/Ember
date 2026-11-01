// Exercise a shell upgrade on a disposable copy of a packaged, single-module site.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { open } from "../../FerroUI/scripts/browser/harness.mjs";

if (!process.argv[2]) throw new Error("Usage: node shell-update.mjs SITE");
const buildManifest = fileURLToPath(new URL("../write-offline-manifest.mjs", import.meta.url));
const folder = fs.mkdtempSync(path.join(os.tmpdir(), "ember-update-"));
const site = path.join(folder, "site");
const initScript = `const get = globalThis.fetch.bind(globalThis);
globalThis.fetch = (input, init) => {
    const url = new URL(input instanceof Request ? input.url : input, location.href);
    return url.hostname === "hacker-news.firebaseio.com" ? Promise.resolve(Response.json([])) : get(input, init);
};`;
let page;
try {
    fs.cpSync(path.resolve(process.argv[2]), site, { recursive: true });
    const initial = JSON.parse(fs.readFileSync(path.join(site, "offline-manifest.json")));
    const modules = Object.keys(initial.moduleHashes);
    assert.equal(modules.length, 1);
    page = await open(site, { width: 390, height: 844, initScript });
    await page.waitFor("!!navigator.serviceWorker.controller");
    const key = await page.evaluate('caches.keys().then(keys => keys.find(k => k.startsWith("ember-wasm:")))');
    assert.ok(key.endsWith(initial.moduleHashes[modules[0]]));
    fs.appendFileSync(path.join(site, "app.css"), "\n/* shell upgrade probe */\n");
    execFileSync(process.execPath, [buildManifest, ".", "--out", site]);
    const next = JSON.parse(fs.readFileSync(path.join(site, "offline-manifest.json")));
    assert.notEqual(next.version, initial.version);
    assert.deepEqual(next.moduleHashes, initial.moduleHashes);

    const bytes = fs.statSync(path.join(site, modules[0])).size;
    fs.renameSync(path.join(site, modules[0]), path.join(folder, "unavailable.wasm"));
    await page.send("Network.clearBrowserCache");
    await page.evaluate("navigator.serviceWorker.getRegistration().then(r => r.update()).then(() => true)");
    await page.waitFor(`caches.keys().then(keys => keys.some(k => k.endsWith(":${next.version}")) && !keys.some(k => k.endsWith(":${initial.version}")))`);
    await page.send("Network.enable");
    await page.send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: 0, uploadThroughput: 0 });
    const origin = await page.evaluate("performance.timeOrigin");
    await page.send("Page.reload", {});
    await page.waitFor(`performance.timeOrigin !== ${origin} && !!globalThis.ember?.emberState && JSON.parse(ember.emberState())?.elements.some(e => e.name === "FeedTitle")`);
    console.log(`Passed: shell updated and reloaded offline with the ${bytes}-byte module unavailable from the server.`);
} finally {
    if (page) await page.close();
    fs.rmSync(folder, { recursive: true, force: true });
}
