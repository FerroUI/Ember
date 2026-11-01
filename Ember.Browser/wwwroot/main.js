import { locationItem } from "./ember-platform.js";

const host = document.getElementById("out");

// Accept the same public discussion ID forms as the reader. Only the decimal ID
// is carried across to the fixed Hacker News origin.
function setSourceLink(link) {
    const id = locationItem(location.href);
    if (id) {
        link.href = "https://news.ycombinator.com/item?id=" + id;
        link.textContent = "Read discussion on Hacker News";
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
    // The build supplies this hint only for the single-module site. Combined
    // builds let FerroUI choose between plain and threaded modules first.
    const wasm = document.querySelector('meta[name="ember-wasm"]');
    if (wasm) {
        const preload = document.createElement("link");
        preload.rel = "preload";
        preload.as = "fetch";
        preload.type = "application/wasm";
        preload.crossOrigin = "anonymous";
        preload.href = wasm.content;
        document.head.append(preload);
        const glue = document.createElement("link");
        glue.rel = "modulepreload";
        glue.href = "./ember-browser.js";
        document.head.append(glue);
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
