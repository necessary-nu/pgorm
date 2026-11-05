# Packaging and distribution

A Rust program takes pgorm through Cargo, and the Rust guide has no chapter
like this one. A Python program takes it as a wheel: a built package holding
the Python modules and the compiled extension. This chapter covers what a
wheel and a source distribution hold, how to build them, which interpreters
and platforms they are for, where a wheel finds the certificates it trusts,
the notices it carries, and how an application's own wheel replaces the
standalone one.

## What a wheel holds

A `pgorm` wheel holds the package's Python modules, the native extension
`pgorm._native`, `py.typed` and the type declarations ([Types and
mypy](25-typing.md)), the licence files, and the notices of what the
extension compiles in. It holds no test fixtures, no security harness and
none of the repository's planning files.

The package has no Python dependencies at run time, and a program that
imports it needs no Rust toolchain. Its metadata says what it needs:

```python
from importlib.metadata import distribution, requires

import pgorm

installed = distribution("pgorm")
assert installed.version == pgorm.__version__
assert installed.metadata["Requires-Python"].replace(" ", "") == ">=3.14,<3.15"
assert installed.metadata["License-Expression"] == "MIT OR Apache-2.0"
assert requires("pgorm") is None
```

## Building a wheel and a source distribution

A wheel is built from a checkout of pgorm's repository with
[maturin](https://www.maturin.rs), as [Getting
started](01-getting-started.md) does. The build needs a Rust toolchain, a C
compiler and libclang, since the extension compiles PostgreSQL's parser,
libpg_query, from its C sources, and CPython 3.14 with maturin 1.15 or later
below 2. `--locked` builds the dependency versions the committed lockfile
holds.

The source distribution holds the Rust crates the extension is built from,
pgorm's own among them, and the lockfile, so it rebuilds the same package
without the rest of the repository. From the repository root:

```sh
.venv/bin/maturin sdist --manifest-path pgorm-python/Cargo.toml --out dist
```

A PEP 517 front end builds both in an isolated environment of its own,
installing maturin from the package's build requirements:

```sh
uv build pgorm-python --python .venv/bin/python --wheel --sdist --out-dir dist \
  --config-setting build-args=--locked
```

A wheel built from the source distribution holds the same Python files and
package metadata as one built from the checkout. The only build option is
the `pg-19` feature, which [PostgreSQL 18 and 19](26-postgres-releases.md)
covers. None of these commands publishes anything.

The package is a Cargo workspace of its own, with its own lockfile. Building
pgorm at the repository root, `cargo build --workspace`, never loads PyO3 or
any Python configuration, and a Rust program that depends on pgorm never
compiles the binding.

## CPython 3.14, with the GIL

A wheel is built for one CPython release, 3.14, and its tag says so
(`cp314`). It is not a stable-ABI wheel, so it loads on 3.14 alone.
Free-threaded CPython and subinterpreters are not supported.
`pgorm.capabilities()` reports the policy, and the target the extension was
compiled for:

```python
import sys

import pgorm

capabilities = pgorm.capabilities()
assert capabilities["python"] == {"abi": "cp314", "free_threading": False, "subinterpreters": False}
assert sys.version_info[:2] == (3, 14)

# The platform the native extension was compiled for, such as
# "aarch64-apple-darwin" or "x86_64-unknown-linux-gnu".
assert isinstance(capabilities["target"], str)
```

## Platforms

A wheel's platform tag names the platform it was built for. The package's
[support.json](../support.json) records the platforms wheels are meant for,
and apart from them the combinations of operating system, architecture and
interpreter whose installation checks have passed. Wheels are meant for two
platforms today: macOS on arm64 and Linux on x86-64. A platform there is a
candidate until a wheel built for it has passed those checks, and only a
combination that has is claimed as supported. The file also states the ABI
policy, the Python versions and the registry status. It is in the
repository and the source distribution, not in the wheel.

How a wheel and a source distribution are checked before a platform is
claimed, and the record of each run, is maintainer material; the checks
themselves are in `pgorm-python/checks/`.

## Which certificates a wheel trusts

A wheel carries no list of certificate authorities. A pool that verifies TLS
without `ca` or `cafile` trusts the roots of the host it runs on, read from
the platform's certificate store each time such a pool is constructed:

- on macOS, the system roots and every certificate the user or an
  administrator has marked trusted in Keychain;
- on Windows, the root certificate store;
- on Linux and other Unix systems, the CA bundle and certificate directory
  OpenSSL would use, such as `/etc/ssl/certs`.

The same wheel therefore trusts different authorities on different hosts,
and follows the host's own additions and removals, such as a corporate CA or
a distrusted root, without a rebuild. Reading the store links the extension
against the Security framework on macOS and the crypt32 library on Windows;
on Linux it reads files and links nothing further.

`SSL_CERT_FILE` and `SSL_CERT_DIR`, when set, replace the platform's store
rather than add to it. A store that holds no usable root certificate, such
as a minimal container image without a CA bundle, or `SSL_CERT_FILE` naming
a missing or empty file, raises `ConstructionError` where the pool is
constructed, naming what was wrong. It never gives a pool that trusts
nothing, or one that skips verification:

```python
import asyncio
import os
import tempfile

from pgorm import ConstructionError, Pool


async def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        empty = os.path.join(directory, "empty.pem")
        open(empty, "w").close()
        os.environ["SSL_CERT_FILE"] = empty

        try:
            Pool("postgres://bakery@db.example.com/bakery")
        except ConstructionError as error:
            assert "SSL_CERT_FILE" in str(error)
        else:
            raise AssertionError("a store with no roots is refused")

        # A plaintext pool reads no store.
        plain = Pool("postgres://bakery@localhost/bakery", tls="disable")
        await plain.close()


asyncio.run(main())
```

A deployment that relies on the host's trust has to provide a CA bundle: on
Debian and Ubuntu images the `ca-certificates` package, which slim and
distroless images can lack, or a bundle named by `SSL_CERT_FILE` or
`SSL_CERT_DIR`. A pool given `ca` or `cafile` trusts exactly those
certificates and never reads the store, and a plaintext pool
(`tls="disable"`, or `sslmode=disable` in the URL) is unaffected.
[Connection configuration and TLS](20-configuration.md) covers the pool's
TLS options.

## Dependency notices and licences

pgorm is licensed MIT OR Apache-2.0. The extension compiles in Rust crates
and native code under licences of their own, and the wheel carries their
notices inside the package:

- `pgorm/THIRD_PARTY_NOTICES.txt`, the full notice texts, which the wheel's
  licence metadata also names;
- `pgorm/DEPENDENCIES.json`, an inventory of every package in the locked
  Cargo graph, with its version, licence, source and the notices it ships,
  and the native components compiled in: libpg_query and its PostgreSQL
  parser, upb, utf8_range and xxHash, and ring's bundled cryptography.

The inventory covers the whole locked graph, build dependencies and
packages only some platforms use included, so it lists more than any one
wheel holds. The system libraries the extension links and the Python
interpreter come from the deployment, not the wheel.

```python
import json
from importlib.resources import files

import pgorm

package = files(pgorm)
inventory = json.loads(package.joinpath("DEPENDENCIES.json").read_text())

assert inventory["python_runtime_dependencies"] == []
assert {"libpg_query", "PostgreSQL parser"} <= {
    component["name"] for component in inventory["bundled_native_components"]
}
assert all(entry["license"] for entry in inventory["packages"])
assert package.joinpath("THIRD_PARTY_NOTICES.txt").is_file()
```

The notice files are committed with the package, so an installed
application reads them without network access.

## Registries

The package is not published to PyPI or any other index yet, and no name
there is reserved for it. Install a wheel you built, or one built for you
from this repository.

## Application wheels

An application that registers its own Rust entities builds a wheel of its
own ([Registering Rust entities](24-registrations.md)). That wheel is the
`pgorm` package again: the same Python modules and declarations, with a
native extension compiled with the application's registrations in, and the
module code generation wrote for them beside the rest. It replaces the
standalone wheel rather than adding to it, since both install as the
distribution `pgorm`: an environment holds one or the other.

The application wheel carries the package's notices as the standalone wheel
does. They inventory the binding's own locked graph; the crates the
application adds are its own to account for.
