// Build the addon and place it where lib/index.js loads it.
//
//   node scripts/build.mjs [--release] [--features pg-19]
//
// The crate is an rlib for the applications that link it, so the addon's
// cdylib is asked of `cargo rustc`. Cargo names a cdylib for the platform
// (libpgorm_napi.dylib, .so, pgorm_napi.dll); Node-API hosts load it from any
// path, so it is copied to lib/pgorm_napi.node. A release build uses the
// crate's `addon` profile, the release profile with whole-program LTO and the
// symbol table stripped, which is what the platform packages ship. Builds go
// to the repository's target directory, which the nplan checks share, unless
// CARGO_TARGET_DIR says otherwise. `--features` passes the crate's features
// on, `pg-19` the one there is, which targets PostgreSQL 19.
// checks/package.js builds the platform packages' addon through
// `buildLibrary`, and copies it nowhere in lib/.

import { spawnSync } from "node:child_process";
import { copyFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

/**
 * Build the addon's cdylib and return the path Cargo wrote it to.
 *
 * @param {{ release?: boolean, features?: string[] }} [options]
 * @returns {string}
 */
export function buildLibrary({ release = false, features = [] } = {}) {
  const args = [
    "rustc",
    "--manifest-path",
    join(root, "Cargo.toml"),
    "--locked",
    "--lib",
    "--crate-type",
    "cdylib",
    "--message-format=json-render-diagnostics",
  ];
  if (release) args.push("--profile", "addon");
  if (features.length > 0) args.push("--features", features.join(","));
  if (!process.env.CARGO_TARGET_DIR) {
    args.push("--target-dir", join(root, "..", "target"));
  }

  const cargo = spawnSync("cargo", args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (cargo.error) throw cargo.error;
  if (cargo.status !== 0) process.exit(cargo.status ?? 1);

  const library = cargo.stdout
    .split("\n")
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line))
    .filter((message) =>
      message.reason === "compiler-artifact" &&
      message.target?.name === "pgorm_napi" &&
      message.target.crate_types.includes("cdylib")
    )
    .flatMap((message) => message.filenames)
    .find((file) => /\.(dylib|so|dll)$/.test(file));
  if (!library) {
    console.error("cargo reported no pgorm_napi cdylib");
    process.exit(1);
  }
  return library;
}

if (import.meta.url === new URL(process.argv[1] ?? "", "file:").href) {
  const named = process.argv.indexOf("--features");
  const features = named === -1 ? [] : (process.argv[named + 1] ?? "").split(",").filter(Boolean);
  const library = buildLibrary({ release: process.argv.includes("--release"), features });
  // Replaced rather than overwritten: macOS kills a process that maps a signed
  // binary whose file changed in place, and a fresh inode sidesteps it.
  const destination = join(root, "lib", "pgorm_napi.node");
  rmSync(destination, { force: true });
  copyFileSync(library, destination);
  console.error(`${library} -> ${destination}`);
}
