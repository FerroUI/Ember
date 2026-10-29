import { FerroExports } from "./ferroui.js";
import { loadModule } from "./ferroui-loader.js";

const host = document.getElementById("out");
try {
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
            // The build wrapper adds offline shell support to the same service
            // worker used by FerroUI's file picker; registrations never compete.
            navigator.serviceWorker.register("./ferroui-sw.js", { updateViaCache: "none" })
                .catch(error => console.warn("Ember's offline shell is unavailable:", error));
        }
    }
} catch (error) {
    console.error(error);
    const panel = document.createElement("section");
    panel.className = "boot-error";
    const title = document.createElement("h1");
    title.textContent = "Ember could not start";
    const detail = document.createElement("p");
    detail.textContent = error instanceof Error ? error.message : String(error);
    const retry = document.createElement("button");
    retry.textContent = "Try again";
    retry.addEventListener("click", () => location.reload());
    panel.append(title, detail, retry);
    host.replaceChildren(panel);
}
