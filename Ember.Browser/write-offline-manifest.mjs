import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

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
if (mode !== "browser") {
    console.log("Ember IndexedDB offline data is enabled. Shell precaching is available in the default non-threaded build.");
    process.exit(0);
}
const files = [];
function visit(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const absolute = path.join(directory, entry.name);
        if (entry.isDirectory()) visit(absolute);
        else if (!entry.name.endsWith(".map") && !["offline-manifest.json", "ferroui-sw.js"].includes(entry.name)) files.push(path.relative(out, absolute).replaceAll(path.sep, "/"));
    }
}
visit(out);
files.sort();
const hash = crypto.createHash("sha256");
for (const file of files) { hash.update(file); hash.update(fs.readFileSync(path.join(out, file))); }
hash.update(fs.readFileSync(path.join(out, "ferroui-sw.js")));
const version = hash.digest("hex").slice(0, 20);
fs.writeFileSync(path.join(out, "offline-manifest.json"), JSON.stringify({ version, files }, null, 2) + "\n");
fs.appendFileSync(path.join(out, "ferroui-sw.js"), `\nself.EMBER_SHELL_VERSION = ${JSON.stringify(version)};\nself.EMBER_SHELL_FILES = ${JSON.stringify(files)};\nimportScripts("./ember-offline.js");\n`);
console.log(`Ember offline shell: ${files.length} files, version ${version}`);
