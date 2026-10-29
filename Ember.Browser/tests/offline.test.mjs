import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

const source = fs.readFileSync(new URL("../wwwroot/ember-offline.js", import.meta.url), "utf8");

function worker({ isolate = false, failure = false } = {}) {
    const listeners = new Map();
    const deleted = [];
    const added = [];
    const requested = [];
    const scope = "https://example.test/ember/";
    const cache = {
        addAll: async files => { added.push(...files); if (failure) throw new Error("QuotaExceededError"); },
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
        self, URL,
        caches: { open: async () => cache, keys: async () => [`ember-shell:${scope}:old`, `ember-shell:${scope}:new`, "different-app:old"], delete: async key => deleted.push(key) },
        fetch: async request => { requested.push(request); return "network"; },
    });
    return { listeners, deleted, added, requested, scope };
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
