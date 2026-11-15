import { readFile } from "node:fs/promises";
import assert from "node:assert/strict";
const read = (path) => readFile(new URL(`../${path}`, import.meta.url), "utf8");
const json = async (path) => JSON.parse(await read(path));
const [pkg, lock, config, cargo, cargoLock, lib, build, main, gadget] = await Promise.all([
  json("package.json"), json("package-lock.json"), json("src-tauri/tauri.conf.json"), read("src-tauri/Cargo.toml"), read("src-tauri/Cargo.lock"),
  read("src-tauri/src/lib.rs"), read("src-tauri/build.rs"), json("src-tauri/capabilities/default.json"), json("src-tauri/capabilities/gadget.json"),
]);
assert.equal(pkg.version, lock.version); assert.equal(pkg.version, lock.packages[""].version); assert.equal(pkg.version, config.version);
assert.equal(pkg.version, cargo.match(/^version = "([^"]+)"/m)[1]);
assert.equal(pkg.version, cargoLock.match(/name = "sonora"\r?\nversion = "([^"]+)"/)[1]);
const handlers = [...lib.split("tauri::generate_handler![")[1].split("]")[0].matchAll(/commands::(\w+)/g)].map((match) => match[1]);
const declared = [...build.matchAll(/^\s+"(\w+)",?$/gm)].map((match) => match[1]);
assert.deepEqual([...handlers].sort(), [...declared].sort(), "Custom commands must be declared in AppManifest");
assert.deepEqual(main.windows, ["main"]); assert.deepEqual(gadget.windows, ["gadget"]);
for (const handler of handlers) assert(main.permissions.includes(`allow-${handler.replaceAll("_", "-")}`), `Missing main permission: ${handler}`);
for (const forbidden of ["allow-get-api-keys", "allow-save-api-keys", "allow-set-transcription-route"]) assert(!gadget.permissions.includes(forbidden));
assert(!cargo.match(/features\s*=\s*\[[^\]]*"devtools"/));
assert(!config.app.security.csp.includes("unsafe-eval"));
assert((await read("README.md")).includes(pkg.version));
console.log(`Versions ${pkg.version}, ${handlers.length} command permissions, window separation and production CSP verified.`);
