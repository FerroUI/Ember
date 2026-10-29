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
            const cache = await caches.open(prefix + self.EMBER_SHELL_VERSION);
            // Either the complete shell is ready, or installation fails and the
            // previously working worker keeps serving its complete version.
            await cache.addAll(files);
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
