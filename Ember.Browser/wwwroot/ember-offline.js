// Appended as an import of FerroUI's existing worker by scripts/build-browser.sh (write-offline-manifest.mjs).
// API responses remain in IndexedDB under the repository's cache policy.
(() => {
    if (new URL(self.location.href).searchParams.get("coi") === "1") return;
    const scope = self.registration.scope;
    const prefix = `ember-shell:${scope}:`;
    const files = self.EMBER_SHELL_FILES.map(file => new URL(file, scope).href);
    const known = new Set(files);
    const index = new URL("index.html", scope).href;
    self.addEventListener("install", event => {
        event.waitUntil((async () => {
            const name = prefix + self.EMBER_SHELL_VERSION;
            const existing = await caches.has(name);
            const cache = await caches.open(name);
            try {
                await cache.addAll(files);
            } catch (error) {
                // Some Chromium installations can fetch a large module but reject
                // its streamed body in Cache Storage. Keep the atomic fast path above.
                const modules = files.filter(file => new URL(file).pathname.endsWith(".wasm"));
                if (error.name !== "NetworkError" || modules.length === 0) throw error;
                try {
                    await cache.addAll(files.filter(file => !modules.includes(file)));
                    for (const file of modules) {
                        const response = await fetch(file);
                        if (response.status !== 200) {
                            throw new Error("Cannot cache the offline module (HTTP " + response.status + ")");
                        }
                        // Fetch decodes HTTP content codings before exposing the bytes.
                        const bytes = await response.arrayBuffer();
                        const headers = new Headers(response.headers);
                        headers.delete("Content-Encoding");
                        headers.set("Content-Length", String(bytes.byteLength));
                        await cache.put(file, new Response(bytes, { headers }));
                    }
                } catch (retryError) {
                    // Activation requires every file. A failed new version must not
                    // leave partial assets or remove a previously installed version.
                    if (!existing) await caches.delete(name);
                    throw retryError;
                }
            }
        })());
    });
    self.addEventListener("activate", event => {
        event.waitUntil((async () => {
            // The version is embedded by the build wrapper. Activation must not
            // require the network after a successfully installed shell.
            const keep = prefix + self.EMBER_SHELL_VERSION;
            for (const key of await caches.keys()) {
                if (key.startsWith(prefix) && key !== keep) await caches.delete(key);
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
            const cache = await caches.open(prefix + self.EMBER_SHELL_VERSION);
            const key = isIndex ? index : request;
            const cached = await cache.match(key, { ignoreSearch: true });
            return cached ?? fetch(request);
        })());
    });
})();
