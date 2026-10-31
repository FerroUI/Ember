const host = document.getElementById("out");

function showError(message, detail) {
    const panel = document.getElementById("startup-error").content.cloneNode(true);
    panel.querySelector("p").textContent = message;
    panel.querySelector("pre").textContent = detail;
    panel.querySelector("button").addEventListener("click", () => location.reload());
    host.replaceChildren(panel);
    host.querySelector("h1").focus();
}

try {
    const [{ FerroExports }, { loadModule }] = await Promise.all([
        import("./ferroui.js"),
        import("./ferroui-loader.js"),
    ]);
    const loaded = await loadModule({ script: "./ember-browser.js", element: host });
    if (loaded) {
        const runtime = await loaded.createRuntime();
        FerroExports.attach(runtime);
        globalThis.ember = runtime;
        runtime.runMain(globalThis.location.href);
        const navigate = () => runtime.emberNavigate(globalThis.location.href);
        globalThis.addEventListener("popstate", navigate);
        globalThis.addEventListener("hashchange", navigate);
        if ("serviceWorker" in navigator && isSecureContext && navigator.onLine !== false && !globalThis.ferrouiModule?.threads) {
            // The offline shell and FerroUI file picker share one worker registration.
            navigator.serviceWorker.register("./ferroui-sw.js", { updateViaCache: "none" })
                .catch(error => console.warn("Ember's offline shell is unavailable:", error));
        }
    } else {
        // On a threaded-only site, null means isolation failed and the loader
        // placed its specific reason and hosting instructions inside the host.
        const reason = host.querySelector("#ferroui-threads-message")?.textContent;
        if (reason) showError("This version needs a browser window that supports isolated threads.", reason);
    }
} catch (error) {
    console.error(error);
    const message = navigator.onLine === false
        ? "Reconnect and reload to finish opening the reader."
        : "Reload to try opening the reader again.";
    showError(message, error instanceof Error ? error.message : String(error));
}
