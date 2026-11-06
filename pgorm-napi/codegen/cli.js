// @ts-check

// pgorm-napi's code generator, from the command line:
//
//   node pgorm-napi/codegen/cli.js scaffold <application.json> --pgorm-source <checkout> --output <project>
//   node pgorm-napi/codegen/cli.js build <project> [--release]
//   node pgorm-napi/codegen/cli.js emit <project>
//   node pgorm-napi/codegen/cli.js models <schema.sql> --output <models.ts> [--schema-name <schema>]
//
// scaffold writes the project's crate and a copy of the binding's ES module;
// build compiles the crate and places its library as the module's addon;
// emit writes the typed module of the application's registrations; models
// writes the declared models of the tables a schema's DDL creates.
// [spec:pgorm:req:napi.codegen+1]

import { spawnSync } from "node:child_process";
import { copyFileSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import process from "node:process";

import { CodegenError, load } from "./config.js";
import { emit } from "./emit.js";
import { manifest, scaffold } from "./scaffold.js";

/**
 * The value after `flag` in `args`, if given.
 *
 * @param {string[]} args
 * @param {string} flag
 */
function option(args, flag) {
  const at = args.indexOf(flag);
  return at < 0 ? undefined : args[at + 1];
}

/**
 * Build the project's crate and copy its library to `lib/pgorm_napi.node`.
 *
 * @param {string} project
 * @param {boolean} release
 */
export function build(project, release) {
  const name = manifest(join(project, "Cargo.toml")).get("lib.name");
  const args = ["build", "--manifest-path", join(project, "Cargo.toml"), "--lib", "--message-format=json-render-diagnostics"];
  if (release) args.push("--release");
  const cargo = spawnSync("cargo", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
  if (cargo.error) throw cargo.error;
  if (cargo.status !== 0) throw new CodegenError(`cargo build exited with ${cargo.status}`);
  const library = cargo.stdout
    .split("\n")
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line))
    .filter((message) =>
      message.reason === "compiler-artifact" && message.target?.name === name &&
      message.target.crate_types?.includes("cdylib")
    )
    .flatMap((message) => message.filenames)
    .find((file) => /\.(dylib|so|dll)$/.test(file));
  if (!library) throw new CodegenError("cargo reported no cdylib");
  const destination = join(project, "lib", "pgorm_napi.node");
  rmSync(destination, { force: true });
  copyFileSync(library, destination);
  return destination;
}

async function main() {
  const [command, path, ...rest] = process.argv.slice(2);
  if (command === "scaffold" && path) {
    const source = option(rest, "--pgorm-source");
    const output = option(rest, "--output");
    if (!source || !output) throw new CodegenError("scaffold takes --pgorm-source and --output");
    console.error(scaffold(load(resolve(path)), output, { source }));
  } else if (command === "build" && path) {
    console.error(build(resolve(path), rest.includes("--release")));
  } else if (command === "emit" && path) {
    console.error(await emit(resolve(path)));
  } else if (command === "models" && path) {
    const output = option(rest, "--output");
    if (!output) throw new CodegenError("models takes --output");
    const { modelsFromSql } = await import("../lib/index.js");
    writeFileSync(output, modelsFromSql(readFileSync(path, "utf8"), { schemaName: option(rest, "--schema-name") ?? null }));
    console.error(resolve(output));
  } else {
    throw new CodegenError(
      "usage: cli.js scaffold <application.json> --pgorm-source <checkout> --output <project> | build <project> [--release] | emit <project> | models <schema.sql> --output <models.ts> [--schema-name <schema>]",
    );
  }
}

if (import.meta.url === new URL(process.argv[1] ?? "", "file:").href) {
  main().catch((error) => {
    console.error(error instanceof CodegenError ? error.message : error);
    process.exit(1);
  });
}
