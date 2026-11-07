import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { prepareCjkFont } from "./prepare-cjk-font.mjs";

const [root, ...args] = process.argv.slice(2);
let out;
let mode = "browser";
let features = "";
for (let index = 0; index < args.length; ++index) {
    const option = args[index];
    if (option === "--out") out = args[++index];
    else if (option === "--features") features = args[++index];
    else if (option === "--threads") mode = "browser-threads";
    else if (option === "--both") mode = "browser-both";
}
const target = process.env.CARGO_TARGET_DIR ?? path.join(root, "target");
out = path.resolve(out ?? path.join(target, mode + (features ? "-" + features.replaceAll(",", "-") : ""), "ember-browser"));
if (args.includes("--package-font")) await prepareCjkFont(out);
if (mode !== "browser") {
    console.log("Ember IndexedDB offline data is enabled. Shell precaching is available in the default non-threaded build.");
    process.exit(0);
}
const files = [];
function visit(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const absolute = path.join(directory, entry.name);
        if (entry.isDirectory()) {
            if (absolute !== path.join(out, "optional-fonts")) visit(absolute);
        }
        else if (!entry.name.endsWith(".map") && !["offline-manifest.json", "ferroui-sw.js"].includes(entry.name)) files.push(path.relative(out, absolute).replaceAll(path.sep, "/"));
    }
}
visit(out);
files.sort();
const modules = files.filter(file => file.endsWith(".wasm"));
const html = path.join(out, "index.html");
if (modules.length === 1 && /^[a-zA-Z0-9_.-]+$/.test(modules[0])) {
    const markup = fs.readFileSync(html, "utf8").replace("<!-- Ember single-module hint -->",
        `<meta name="ember-wasm" content="./${modules[0]}">`);
    fs.writeFileSync(html, markup);
}
const hash = crypto.createHash("sha256");
const moduleHashes = {};
for (const file of files) {
    hash.update(file);
    const moduleHash = modules.includes(file) ? crypto.createHash("sha256") : null;
    for await (const chunk of fs.createReadStream(path.join(out, file))) {
        hash.update(chunk);
        moduleHash?.update(chunk);
    }
    if (moduleHash) moduleHashes[file] = moduleHash.digest("base64");
}
const worker = path.join(out, "ferroui-sw.js");
// Rebuilding the manifest for an existing site must not recursively hash or
// append the previous version's generated worker configuration.
const baseWorker = fs.readFileSync(worker, "utf8").split("\nself.EMBER_SHELL_VERSION = ")[0];
hash.update(baseWorker);
const version = hash.digest("hex").slice(0, 20);
fs.writeFileSync(path.join(out, "offline-manifest.json"), JSON.stringify({ version, files, moduleHashes }, null, 2) + "\n");
fs.writeFileSync(worker, baseWorker + `\nself.EMBER_SHELL_VERSION = ${JSON.stringify(version)};\nself.EMBER_SHELL_FILES = ${JSON.stringify(files)};\nself.EMBER_MODULE_HASHES = ${JSON.stringify(moduleHashes)};\nimportScripts("./ember-offline.js");\n`);
console.log(`Ember offline shell: ${files.length} files, version ${version}`);
