import {execFile} from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import {promisify} from "node:util";
import { cjkFontAsset } from "./wwwroot/ember-platform.js";

const run = promisify(execFile);
const upstream = "https://raw.githubusercontent.com/notofonts/noto-cjk/523d033d6cb47f4a80c58a35753646f5c3608a78/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf";

function matches(bytes) {
    return bytes.length === cjkFontAsset.size && crypto.createHash("sha256").update(bytes).digest("hex") === cjkFontAsset.sha256;
}

// The optional payload belongs to the published site. It is intentionally absent
// from git, the Wasm module and the service worker's installation transaction.
export async function prepareCjkFont(out) {
    const target = path.join(out, cjkFontAsset.path);
    await fs.mkdir(path.dirname(target), {recursive: true});
    for (const name of ["OFL.txt", "NOTICE.txt"]) {
        const source = new URL("./wwwroot/optional-fonts/" + name, import.meta.url);
        await fs.copyFile(source, path.join(path.dirname(target), name));
    }
    try {
        if (matches(await fs.readFile(target))) return;
    } catch (error) {
        if (error.code !== "ENOENT") throw error;
    }
    const temporary = target + "." + crypto.randomUUID() + ".tmp";
    try {
        if (process.env.EMBER_CJK_FONT) {
            await fs.copyFile(process.env.EMBER_CJK_FONT, temporary);
        } else {
            // Use the browser toolchain's curl, including its proxy and system CA configuration.
            await run("curl", ["--fail", "--silent", "--show-error", "--proto", "=https",
                "--retry", "2", "--max-time", "120", "--max-filesize", String(cjkFontAsset.size),
                "--output", temporary, upstream]);
        }
        if (!matches(await fs.readFile(temporary))) throw new Error("Noto Sans CJK has an unexpected size or SHA-256 digest");
        await fs.rename(temporary, target);
    } finally {
        await fs.rm(temporary, {force: true});
    }
    console.log("Packaged optional Noto Sans CJK SC Regular (" + cjkFontAsset.size + " bytes)");
}
