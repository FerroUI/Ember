import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createFontLoader } from "../wwwroot/ember-platform.js";

const moduleUrl = "https://example.test/deployment/ember-platform.js";
const bytes = Buffer.from("OTTO test font bytes");
const asset = {path: "optional-fonts/test.otf", size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex")};
const url = new URL(asset.path, moduleUrl).href;
const cacheName = "ember-fonts:https://example.test/deployment/";

function storage(t, {denied = false, full = false} = {}) {
    const responses = new Map();
    const opened = [];
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, "caches");
    Object.defineProperty(globalThis, "caches", {configurable: true, value: {
        open: async name => {
            opened.push(name);
            if (denied) throw new Error("Storage access denied");
            return {
                match: async url => responses.get(url)?.clone(),
                put: async (url, response) => {
                    if (full) throw new Error("Quota exceeded");
                    responses.set(url, response);
                },
                delete: async key => responses.delete(typeof key === "string" ? key : key.url),
                keys: async () => Array.from(responses.keys(), url => ({url})),
            };
        },
    }});
    t.after(() => { if (descriptor) Object.defineProperty(globalThis, "caches", descriptor); else delete globalThis.caches; });
    return {opened, responses};
}

test("font loading is lazy, coalesces misses, and reuses verified bytes across visits offline", async t => {
    const {opened, responses} = storage(t);
    let requests = 0;
    let online = true;
    t.mock.method(globalThis, "fetch", async (requested, options) => {
        requests++;
        assert.equal(requested, url);
        assert.equal(options.credentials, "omit");
        assert.equal(options.referrerPolicy, "no-referrer");
        assert.equal(options.mode, "same-origin");
        assert.equal(options.redirect, "error");
        if (!online) throw new Error("offline");
        return new Response(bytes);
    });
    await import("../wwwroot/ember-platform.js?fresh-font-module");
    const load = createFontLoader(asset, moduleUrl);
    assert.equal(requests, 0);
    assert.deepEqual(opened, []);
    const first = load();
    assert.equal(load(), first);
    assert.deepEqual(Buffer.from(await first), bytes);
    assert.equal(requests, 1);
    assert.deepEqual([...responses.keys()], [url]);
    online = false;
    assert.deepEqual(Buffer.from(await createFontLoader(asset, moduleUrl)()), bytes);
    assert.equal(requests, 1);
    assert.deepEqual(opened, [cacheName, cacheName]);
});

test("a corrupt cached font and invalid network bytes cannot render or replace a previous digest", async t => {
    const {responses} = storage(t);
    const previous = url.replace("test.otf", "earlier.otf");
    responses.set(previous, new Response(bytes));
    const corrupt = Buffer.from(bytes);
    corrupt[corrupt.length - 1] ^= 1;
    responses.set(url, new Response(corrupt));
    const attempts = [
        new Response(corrupt),
        new Response(bytes.subarray(0, bytes.length - 1)),
        new Response(Buffer.concat([bytes, bytes])),
        new Response(bytes, {status: 206}),
        new Response(new Uint8Array(bytes.length)),
        new Response(bytes),
    ];
    t.mock.method(globalThis, "fetch", async () => {
        const response = attempts.shift();
        assert.ok(response, "unexpected fetch");
        return response;
    });
    const load = createFontLoader(asset, moduleUrl);
    for (const reason of [/integrity/, /incomplete/, /size/, /unavailable/, /invalid/]) {
        await assert.rejects(load(), reason);
        assert.deepEqual([...responses.keys()], [previous]);
    }
    assert.deepEqual(Buffer.from(await load()), bytes);
    assert.deepEqual([...responses.keys()], [url]);
});

test("storage denial and quota failures still allow verified font rendering", async t => {
    for (const options of [{denied:true}, {full:true}]) {
        await t.test(JSON.stringify(options), async t => {
            storage(t, options);
            t.mock.method(globalThis, "fetch", async () => new Response(bytes));
            assert.deepEqual(Buffer.from(await createFontLoader(asset, moduleUrl)()), bytes);
        });
    }
});

test("a transient network failure allows a later request to retry", async t => {
    storage(t);
    let offline = true;
    t.mock.method(globalThis, "fetch", async () => {
        if (offline) throw new TypeError("offline");
        return new Response(bytes);
    });
    const load = createFontLoader(asset, moduleUrl);
    await assert.rejects(load(), /offline/);
    offline = false;
    assert.deepEqual(Buffer.from(await load()), bytes);
});

test("optional faces and their notices stay outside the installed offline shell", async t => {
    const site = await fs.mkdtemp(path.join(os.tmpdir(), "ember-font-shell-"));
    t.after(() => fs.rm(site, {recursive:true, force:true}));
    await fs.mkdir(path.join(site, "optional-fonts"));
    await fs.writeFile(path.join(site, asset.path), bytes);
    await fs.writeFile(path.join(site, "optional-fonts", "OFL.txt"), "fixture license");
    await fs.writeFile(path.join(site, "ferroui-sw.js"), "// base worker");
    await fs.writeFile(path.join(site, "index.html"), "<html></html>");
    const manifest = fileURLToPath(new URL("../write-offline-manifest.mjs", import.meta.url));
    execFileSync(process.execPath, [manifest, ".", "--out", site]);
    const result = JSON.parse(await fs.readFile(path.join(site, "offline-manifest.json"), "utf8"));
    assert.deepEqual(result.files, ["index.html"]);
});

test("packaging rejects an invalid offline font mirror without publishing a partial asset", async t => {
    const directory = await fs.mkdtemp(path.join(os.tmpdir(), "ember-font-mirror-"));
    t.after(() => fs.rm(directory, {recursive:true, force:true}));
    const mirror = path.join(directory, "mirror.otf");
    const site = path.join(directory, "site");
    await fs.writeFile(mirror, bytes);
    const packager = fileURLToPath(new URL("../write-offline-manifest.mjs", import.meta.url));
    assert.throws(() => execFileSync(process.execPath, [packager, ".", "--package-font", "--out", site], {
        env: {...process.env, EMBER_CJK_FONT: mirror}, stdio: "pipe",
    }), /unexpected size or SHA-256 digest/);
    assert.deepEqual((await fs.readdir(path.join(site, "optional-fonts"))).sort(), ["NOTICE.txt", "OFL.txt"]);
});
