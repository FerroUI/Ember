const host = document.getElementById("out");

// Accept the same public discussion ID forms as the reader. Only the decimal ID
// is carried across to the fixed Hacker News origin.
function setSourceLink(link) {
    for (const part of location.href.split(/[?&#]/)) {
        const match = /^(?:item=|id=|\/?item\/)([0-9]+)$/.exec(part);
        if (!match) continue;
        const id = BigInt(match[1]);
        if (id === 0n || id > 0xffffffffffffffffn) continue;
        link.href = "https://news.ycombinator.com/item?id=" + id;
        link.textContent = "Read discussion on Hacker News";
        return;
    }
}
setSourceLink(host.querySelector(".boot-link"));

function showError(message, detail) {
    const panel = document.getElementById("startup-error").content.cloneNode(true);
    panel.querySelector("p").textContent = message;
    setSourceLink(panel.querySelector(".boot-link"));
    panel.querySelector("pre").textContent = detail;
    panel.querySelector("button").addEventListener("click", () => location.reload());
    host.replaceChildren(panel);
    host.querySelector("h1").focus();
}

async function startReader() {
    if (typeof WebAssembly !== "object") {
        showError("Open Ember in a browser with WebAssembly enabled, or continue on Hacker News.",
            "WebAssembly is unavailable in this browser.");
        return;
    }
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
        showError("This version needs a browser window that supports isolated threads.", reason || "The runtime could not start isolated threads.");
    }
}

try {
    await startReader();
} catch (error) {
    console.error(error);
    const message = navigator.onLine === false
        ? "Reconnect and reload to finish opening the reader."
        : "Reload to try again, or continue on Hacker News.";
    showError(message, error instanceof Error ? error.message : String(error));
}
