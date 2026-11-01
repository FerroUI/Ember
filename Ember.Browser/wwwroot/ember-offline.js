// Appended as an import of FerroUI's existing worker by scripts/build-browser.sh (write-offline-manifest.mjs).
// API responses remain in IndexedDB under the repository's cache policy.
(() => {
    if (new URL(self.location.href).searchParams.get("coi") === "1") return;
    const scope = self.registration.scope;
    const prefix = `ember-shell:${scope}:`;
    const files = self.EMBER_SHELL_FILES.map(file => new URL(file, scope).href);
    const known = new Set(files);
    const index = new URL("index.html", scope).href;
    const shellName = prefix + self.EMBER_SHELL_VERSION;
    const modulePrefix = `ember-wasm:${scope}:`;
    const modules = new Map();
    for (const path of self.EMBER_SHELL_FILES.filter(file => file.endsWith(".wasm"))) {
        const digest = self.EMBER_MODULE_HASHES?.[path];
        modules.set(new URL(path, scope).href, {
            name: digest ? modulePrefix + digest : shellName,
            integrity: digest ? "sha256-" + digest : "",
        });
    }

    async function cacheModule(cache, file, integrity) {
        // A style or JavaScript-only update can use the exact previously installed
        // module in place. Check the key without opening or copying its body.
        if ((await cache.keys(file)).length) return;
        const request = new Request(file, { integrity });
        try {
            await cache.add(request);
        } catch (error) {
            // Chromium can reject a streamed cache write for a large module.
            if (error.name !== "NetworkError") throw error;
            const response = await fetch(request);
            if (response.status !== 200) {
                throw new Error("Cannot cache the offline module (HTTP " + response.status + ")");
            }
            const bytes = await response.arrayBuffer();
            const headers = new Headers(response.headers);
            headers.delete("Content-Encoding");
            headers.set("Content-Length", String(bytes.byteLength));
            await cache.put(file, new Response(bytes, { headers }));
        }
    }

    self.addEventListener("install", event => {
        event.waitUntil((async () => {
            const created = new Set();
            async function open(name) {
                if (!await caches.has(name)) created.add(name);
                return caches.open(name);
            }
            try {
                const shell = await open(shellName);
                await shell.addAll(files.filter(file => !modules.has(file)));
                for (const [file, { name, integrity }] of modules) {
                    await cacheModule(name === shellName ? shell : await open(name), file, integrity);
                }
            } catch (error) {
                // Leave caches belonging to the active worker intact on a failed upgrade.
                for (const name of created) await caches.delete(name);
                throw error;
            }
        })());
    });
    self.addEventListener("activate", event => {
        event.waitUntil((async () => {
            const keep = new Set([shellName, ...Array.from(modules.values(), module => module.name)]);
            for (const key of await caches.keys()) {
                if ((key.startsWith(prefix) || key.startsWith(modulePrefix)) && !keep.has(key)) await caches.delete(key);
            }
        })());
    });
    self.addEventListener("fetch", event => {
        const request = event.request;
        if (request.method !== "GET" || !request.url.startsWith(scope)) return;
        const url = new URL(request.url);
        url.search = "";
        url.hash = "";
        const isIndex = request.mode === "navigate" && (url.href === scope || url.href === index);
        // Restrict interception to this exact build's files. In particular the
        // platform's generated download URLs belong to its existing handler.
        if (!isIndex && !known.has(url.href)) return;
        event.respondWith((async () => {
            const cache = await caches.open(modules.get(url.href)?.name ?? shellName);
            const key = isIndex ? index : request;
            const cached = await cache.match(key, { ignoreSearch: true });
            return cached ?? fetch(request);
        })());
    });
})();
