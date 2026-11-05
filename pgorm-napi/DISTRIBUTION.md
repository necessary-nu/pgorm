# Distribution

pgorm-napi is distributed as npm packages with prebuilt native addons, the
usual layout for a Node-API module:

- **`@necessary/pgorm`**, the main package: the ES module (`lib/*.js`), its
  TypeScript declarations (`lib/*.d.ts`), this file and `support.json`. It
  holds no native code.
- **`@necessary/pgorm-<os>-<cpu>[-<libc>]`**, one per platform: that
  platform's release build of the addon, `pgorm_napi.node`, and the notices of
  the Rust dependencies compiled into it. The name uses Node.js's
  `process.platform` and `process.arch`, plus the C library on Linux (`gnu`
  for glibc, `musl`), e.g. `@necessary/pgorm-darwin-arm64` or
  `@necessary/pgorm-linux-x64-gnu`. Each declares the `os`, `cpu` and, on
  Linux, `libc` it runs on.

The main package lists the platform packages a release builds as optional
dependencies, so npm and Deno install only the one that matches the machine.
Each one is pinned to the main package's own version. That is the one exact
version requirement the packages carry: the module and the addon are released
together, and a range could pair a module with an addon it was not built for.
Your own project depends on `@necessary/pgorm` with a range as usual, and its
lockfile pins the exact versions.

The Rust crate they are built from keeps its name, `pgorm-napi`. Nothing has
been published yet ([Publishing](#publishing)).

## Installing

pgorm-napi needs a runtime with `Temporal` as a global: Node.js 26 or later, or
Deno 2.9.5 or later.

**Node.js**

```sh
npm install @necessary/pgorm
```

```js
import { connect } from "@necessary/pgorm";

await using pool = await connect(process.env.DATABASE_URL);
console.log(await pool.one("SELECT $1::int8 + 1 AS n", [41n])); // { n: 42n }
```

**Deno**, from npm with a `node_modules` directory that Deno manages:

```json
{
  "nodeModulesDir": "auto",
  "imports": { "@necessary/pgorm": "npm:@necessary/pgorm@^0.2.0" }
}
```

```sh
deno install
deno run --allow-ffi --allow-read app.ts
```

`import { connect } from "@necessary/pgorm"` then resolves through the import
map, and `import { connect } from "npm:@necessary/pgorm@^0.2.0"` works directly
too. Deno installs the matching platform package as it installs any optional
dependency. `nodeModulesDir: "auto"` is the configuration the packaging check
tests.

### The `--allow-ffi` trust point

Deno has to be given `--allow-ffi` before it will open a native library, and
`--allow-read` so it can find the addon. **Native code runs outside Deno's
permission system.** Once a program is allowed to load the addon, the addon
can do anything the process can: open any network connection, read and write
any file, read the environment and start processes, whatever `--allow-net`,
`--allow-read` or `--deny-*` say. pgorm-napi connects to PostgreSQL this way,
which is why it does not need `--allow-net`, and why `--allow-net` does not
limit what it can reach. `--allow-ffi=<path>` limits which libraries may be
opened, not what they do once open. Giving a program `--allow-ffi` therefore
trusts the addon and its dependencies as fully as running it under Node.js.
The notices name what is compiled in, and the [packaging check](#build-and-verify)
proves that what is packed is the release build of this repository's source.

### Platforms

`support.json` records every platform the packages are named for, which ones
a release builds, and which have been tested:

| Platform | Package | Release | Packaging check passed on |
| --- | --- | --- | --- |
| macOS arm64 | `@necessary/pgorm-darwin-arm64` | yes; CI builds it on `macos-15` | macOS 15 in CI and macOS 26.5.1 locally; Node.js 26.11.1; Deno 2.9.7 and 2.9.5 |
| Linux x86-64, glibc | `@necessary/pgorm-linux-x64-gnu` | yes; CI builds it on `ubuntu-24.04` | Ubuntu 24.04 in CI; Node.js 26.11.1; Deno 2.9.7 |
| Linux arm64, glibc | `@necessary/pgorm-linux-arm64-gnu` | no | untested |
| Linux x86-64, musl | `@necessary/pgorm-linux-x64-musl` | no | untested |
| Linux arm64, musl | `@necessary/pgorm-linux-arm64-musl` | no | untested |
| macOS x86-64 | `@necessary/pgorm-darwin-x64` | no | untested |
| Windows x86-64 | `@necessary/pgorm-win32-x64` | no | untested |

Configuring a CI job does not establish a result. A combination moves into
`support.json`'s `tested_combinations`, and into the last column here, only
after the packaging check has passed on it.

Only the release platforms are listed as optional dependencies, so the main
package never depends on a name that nothing publishes. On any other platform,
importing the module throws an error that names the platform it detected and
the platforms that do have packages. The same thing happens, with different
advice, when the platform's package is missing (e.g. installed with
`npm install --omit=optional`), or when its version differs from the main
package's. You never get a failure later, on first use. On Linux, the C
library is the runtime's own: under Node.js the diagnostic report says whether
the process runs on glibc, and Deno, which needs `--allow-sys` for that
report, gives its build environment instead.

To run on another platform, build the addon from a checkout. The module loads
`lib/pgorm_napi.node` when it is present, before looking for a platform
package:

```sh
node pgorm-napi/scripts/build.mjs --release
```

then import `pgorm-napi/lib/index.js` from the checkout.

## Build and verify

`checks/package.js` builds, packs and proves the packages, as
`pgorm-python/checks/distribution.py` does for the wheel. With a PostgreSQL 18
server in `PGORM_TEST_DSN` or `DATABASE_URL` (see the [README](README.md#tests)):

```sh
node pgorm-napi/checks/package.js all --partial
```

It runs three steps, which CI runs on separate runners:

1. **`build`**: builds the release addon for the machine it runs on, with
   the crate's `addon` profile ([The addon's size](#the-addons-size)), into
   `target/napi-package/binaries/<platform>/`. It refuses the build if it
   carries the debug-only exports `probePanic` or `probeDropQueue`, checking
   both by loading the addon and by searching its bytes for their names; if
   it still carries symbols its loading does not need, an ELF symbol table or
   Mach-O local symbols, read from its bytes; or if its `version` is not the
   package's. The npm version always matches the crate's.
2. **`pack`**: checks that the committed notices match the lockfile, checks
   each addon again (by its bytes, so any platform's binary can be checked on
   any runner), stages the main package and one package per release platform,
   runs `npm pack` on each into `target/napi-package/tarballs/`, and checks
   each tarball's file list: no addon in the main package, exactly the addon,
   notices, licences, README and manifest in a platform package. A release
   needs every release platform. `--partial` packs only the platforms built
   on this machine, and the main package then names only those.
3. **`install`**: serves the tarballs from an npm registry it runs on
   loopback, which returns 404 for every other name. Into fresh projects it
   runs `npm install @necessary/pgorm` and, in a `nodeModulesDir: "auto"` Deno
   project, installs `npm:@necessary/pgorm`, each with its own cache and no
   user configuration. It checks that only the matching platform package was
   installed, then runs `tests/package/smoke.test.ts` under `node --test` and
   `deno test`. The suite checks that the installed addon is a stripped
   release build of this version, then covers connecting, a bound query, a
   built statement, a model, a transaction that commits and one that rolls
   back, `Temporal`, `bigint` and `Decimal` round trips, and a fresh process
   that has to exit on its own. In
   both runtimes it then makes the loader refuse a missing platform package,
   one with a different version, and a platform the main package does not
   offer, each with its error.

`target/napi-package/report.json` stays `"passed": false` until `install` has
finished. Nothing is uploaded anywhere.

The [Node-API workflow](../.github/workflows/napi.yml) runs the same steps
on every push to `main` that touches pgorm-napi, Rust or Cargo files, and
weekly. Its `addon` jobs build on `macos-15` (arm64) and `ubuntu-24.04`
(x86-64). Its `packages` job packs the full release set on Linux. Its
`install` jobs install that one set on both platforms against PostgreSQL 18
and run the suite in Node.js 26 and Deno 2. The tarballs, the addons and each
install's report are kept as workflow artifacts for 14 days.

## The addon's size

The packages ship the addon built with pgorm-napi's `addon` Cargo profile,
which `node pgorm-napi/scripts/build.mjs --release` and the packaging check's
`build` step both use:

```toml
[profile.addon]
inherits = "release"
lto = "fat"
codegen-units = 1
strip = "symbols"
```

Fat LTO across one codegen unit lets the optimiser see the whole addon at
once and drop the code nothing reaches. Stripping removes the symbol table,
which no runtime reads: a Node-API host looks up only
`napi_register_module_v1`, which stays exported. Panics still unwind, as in
every other build, because Neon turns a panic into a JavaScript exception
where `panic = "abort"` would end the host process. The `release` profile
itself is unchanged, in pgorm-napi and in the main workspace.

Measured on macOS arm64 (rustc 1.99.0), `pgorm_napi.node` built from one
commit under each setting, then stripped with `strip -x`:

| Profile | As built | Stripped |
| --- | --- | --- |
| Cargo's `release` (16 codegen units, no cross-crate LTO) | 23.2 MB | 18.4 MB |
| `release` with fat LTO | 17.8 MB | 14.6 MB |
| `release` with one codegen unit | 14.9 MB | 12.1 MB |
| `release` with fat LTO and one codegen unit: **`addon`** | 14.1 MB | **11.8 MB** |
| `addon` with `opt-level = "s"` | 11.5 MB | 9.1 MB |
| `addon` with `opt-level = "z"` | 10.4 MB | 6.8 MB |

A release build carries no debug information to begin with, since Cargo
strips it when `debug` is off; what stripping removes is the symbol table,
about 55,000 local symbols. One codegen unit buys more than LTO does, because
sixteen units each carry their own copies of the generic and inlined code
they use.

The profile keeps the release profile's `opt-level = 3`. Counted by
instructions retired and CPU cycles on macOS, the `addon` build decodes rows
in about 4% fewer cycles than the `release` build. `opt-level = "s"` would
give that back for another 2.6 MB, and `"z"` spends about 40% more cycles
decoding rows and nearly twice as many compiling a pipeline.

Packed, the macOS arm64 platform package is 5.3 MB, where it was 8.3 MB with
the `release` build. The `build` step records the addon's size in its
`binary.json`, and each CI `addon` job writes it to the job summary. CI's
Linux x86-64 addon was 28.7 MB built with the `release` profile, 21.2 MB of it
left once stripped.

## Dependencies and notices

The packages have no JavaScript dependencies apart from the platform packages.
`notices/DEPENDENCIES.json` lists every Rust package in the locked Cargo graph,
including build and platform-conditional dependencies, so it covers more than
any one addon contains, with its licence and the hashes of its notice texts.
`notices/THIRD_PARTY_NOTICES.txt` holds those texts. Both files are copied
into every platform package. The inventory also names the C components bundled
through pg_query (libpg_query, PostgreSQL's parser, upb, utf8_range and xxHash)
and ring's bundled cryptography. After a dependency update, run:

```sh
node pgorm-napi/checks/notices.js --write
node pgorm-napi/checks/notices.js
```

`licenses/supplemental.json` supplies, pinned to a source URL and hash, the
upstream notices that some crate archives leave out (neon's among them). A
missing or changed text fails the check. The
[supply-chain workflow](../.github/workflows/supply-chain.yml) audits
pgorm-napi's lockfile with `cargo deny` against the repository's `deny.toml`,
as it does every committed lockfile.

The addon trusts the host's certificate store for TLS (through
rustls-native-certs) unless a pool is given a `ca`. No list of certificate
authorities is bundled; [Connection configuration and
TLS](guide/20-configuration.md) has the details.

## Publishing

Nothing has been published, and no workflow publishes. A release would take:

1. The `@necessary` scope owned on npm by the account or organisation that
   publishes. `support.json`'s `npm_package` and the checkout's `package.json`
   name are the main package's, and each platform package's name is derived
   from it. The packed manifests carry `publishConfig.access: public`, since
   npm would otherwise publish a scoped package as restricted.
2. Every release platform's package published before the main package, at the
   same version, from one run's `packages` artifact (`npm publish <tarball>`
   for each). Deno will not install the main package while any optional
   dependency it lists is missing from the registry, so the whole set has to
   be there.
3. Credentials the repository does not have yet: an `NPM_TOKEN` secret, or
   npm trusted publishing configured for a manual-dispatch workflow, which
   would also give the tarballs provenance.
