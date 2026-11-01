import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

const source = fs.readFileSync(new URL("../wwwroot/ember-offline.js", import.meta.url), "utf8");

function worker({ isolate = false, failure = false, streamFailure = false, moduleStatus = 200, existing = false } = {}) {
    const listeners = new Map();
    const deleted = [];
    const added = [];
    const requested = [];
    const scope = "https://example.test/ember/";
    const writes = new Map();
    const cache = {
        addAll: async files => {
            if (failure) throw new DOMException("QuotaExceededError", "QuotaExceededError");
            if (streamFailure && files.some(file => file.endsWith(".wasm"))) {
                throw new DOMException("Streamed module could not be cached", "NetworkError");
            }
            added.push(...files);
        },
        put: async (file, response) => { writes.set(file, response); },
        match: async key => typeof key === "string" && key.endsWith("index.html") ? "cached index" : undefined,
    };
    const self = {
        location: { href: scope + "ferroui-sw.js" + (isolate ? "?coi=1" : "") },
        registration: { scope },
        EMBER_SHELL_VERSION: "new",
        EMBER_SHELL_FILES: ["index.html", "app.css", "ember-browser.wasm"],
        addEventListener: (type, callback) => listeners.set(type, callback),
    };
    vm.runInNewContext(source, {
        self, URL, Headers, Response,
        caches: { has: async () => existing, open: async () => cache, keys: async () => [`ember-shell:${scope}:old`, `ember-shell:${scope}:new`, "different-app:old"], delete: async key => deleted.push(key) },
        fetch: async request => {
            requested.push(request);
            if (streamFailure) return new Response(new Uint8Array([0, 97, 115, 109]), {
                status: moduleStatus, headers: { "Content-Type": "application/wasm", "Content-Encoding": "gzip" },
            });
            return "network";
        },
    });
    return { listeners, deleted, added, requested, scope, writes };
}

test("offline install waits for the complete version and removes only this scope's obsolete caches", async () => {
    const instance = worker();
    let pending;
    instance.listeners.get("install")({ waitUntil: promise => { pending = promise; } });
    await pending;
    assert.equal(instance.added.length, 3);
    instance.listeners.get("activate")({ waitUntil: promise => { pending = promise; } });
    await pending;
    assert.deepEqual(instance.deleted, [`ember-shell:${instance.scope}:old`]);
});

test("failed shell precache rejects installation, preserving the preceding worker", async () => {
    const instance = worker({ failure: true });
    let pending;
    instance.listeners.get("install")({ waitUntil: promise => { pending = promise; } });
    await assert.rejects(pending, /QuotaExceededError/);
});

test("offline root with an item query uses the application shell, downloads and APIs are untouched", async () => {
    const instance = worker();
    let pending;
    const dispatch = request => {
        pending = undefined;
        instance.listeners.get("fetch")({ request, respondWith: promise => { pending = promise; } });
        return pending;
    };
    assert.equal(await dispatch({ method: "GET", mode: "navigate", url: instance.scope + "?item=8863" }), "cached index");
    assert.equal(dispatch({ method: "GET", mode: "navigate", url: instance.scope + "generated-download-123" }), undefined);
    assert.equal(dispatch({ method: "GET", mode: "cors", url: "https://hacker-news.firebaseio.com/v0/item/8863.json" }), undefined);
    assert.equal(dispatch({ method: "POST", mode: "cors", url: instance.scope + "index.html" }), undefined);
});

test("the isolation worker owns requests in threaded mode", () => {
    assert.equal(worker({ isolate: true }).listeners.size, 0);
});

test("a streamed cache failure retries the complete shell with decoded Wasm bytes", async () => {
    const instance = worker({ streamFailure: true });
    let install;
    instance.listeners.get("install")({ waitUntil: promise => { install = promise; } });
    await install;
    const wasm = instance.scope + "ember-browser.wasm";
    assert.deepEqual(instance.added, [instance.scope + "index.html", instance.scope + "app.css"]);
    assert.deepEqual(instance.requested, [wasm]);
    const response = instance.writes.get(wasm);
    assert.equal(response.headers.get("Content-Type"), "application/wasm");
    assert.equal(response.headers.get("Content-Encoding"), null);
    assert.equal(response.headers.get("Content-Length"), "4");
    assert.deepEqual([...new Uint8Array(await response.arrayBuffer())], [0, 97, 115, 109]);
});

test("an incomplete module rejects installation and cleans only a newly created cache", async () => {
    for (const existing of [false, true]) {
        const instance = worker({ streamFailure: true, moduleStatus: 206, existing });
        let install;
        instance.listeners.get("install")({ waitUntil: promise => { install = promise; } });
        await assert.rejects(install, /HTTP 206/);
        assert.equal(instance.writes.size, 0);
        assert.deepEqual(instance.deleted, existing ? [] : [`ember-shell:${instance.scope}:new`]);
    }
});
