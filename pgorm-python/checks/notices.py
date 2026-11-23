"""Generate or verify the distribution's locked Rust dependency notices."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


# pgsql-rs's crates share the two license files at its workspace root —
# `LICENSE`, libpg_query's BSD-3-Clause for what is ported from it, and
# `LICENSE-PostgreSQL` for the scanner, grammar and PL/pgSQL ported from the
# server — and six of them declare no license, which they are read under as
# deny.toml's `[[licenses.clarify]]` reads them.
PGSQL_RS = "git+https://github.com/necessary-nu/pgsql-rs.git"
PGSQL_RS_FILES = ("LICENSE", "LICENSE-PostgreSQL")
PGSQL_RS_LICENSE = "BSD-3-Clause AND PostgreSQL"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def from_pgsql_rs(package):
    return (package["source"] or "").startswith(PGSQL_RS)


def license_of(package):
    if not package["license"] and from_pgsql_rs(package):
        return PGSQL_RS_LICENSE
    return package["license"]


def notice_paths(package, root):
    directory = Path(package["manifest_path"]).parent
    # Registry packages are bounded archives. Local crates share repository
    # notices; never recurse through the workspace and its build outputs.
    paths = directory.rglob("*") if package["source"] else directory.iterdir()
    result = [
        path
        for path in paths
        if path.is_file()
        and (
            path.name.lower().startswith(
                ("license", "licence", "copying", "copyright", "notice", "unlicense")
            )
            or path.name == "AUTHORS"
            or any(
                part.lower() in ("licenses", "licences")
                for part in path.relative_to(directory).parts
            )
        )
    ]
    if not result and not package["source"]:
        result = [root / "LICENSE-APACHE", root / "LICENSE-MIT"]
    return sorted(result)


def texts_for(package, root, supplements):
    texts = []
    directory = Path(package["manifest_path"]).parent
    for path in notice_paths(package, root):
        data = path.read_bytes()
        # The path inside the package, so a crate vendoring several projects'
        # LICENSE files names which is which.
        name = (
            path.relative_to(directory) if path.is_relative_to(directory) else path.name
        )
        texts.append((str(name), data))
    for entry in supplements.get(f"{package['name']}@{package['version']}", []):
        data = (root / "pgorm-python/licenses" / entry["file"]).read_bytes()
        if digest(data) != entry["sha256"]:
            raise RuntimeError("supplemental license hash differs: " + entry["file"])
        texts.append((entry["source"], data))
    if from_pgsql_rs(package):
        workspace = directory.parents[1]
        for name in PGSQL_RS_FILES:
            texts.append(("pgsql-rs/" + name, (workspace / name).read_bytes()))
    if not texts or not license_of(package):
        raise RuntimeError(
            f"missing license evidence for {package['name']} {package['version']}"
        )
    return texts


def generate(root):
    metadata = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--manifest-path",
                str(root / "pgorm-python/Cargo.toml"),
                "--format-version",
                "1",
                "--locked",
            ],
            text=True,
        )
    )
    supplements = json.loads(
        (root / "pgorm-python/licenses/supplemental.json").read_text()
    )
    packages, notices = [], {}
    for package in sorted(
        metadata["packages"], key=lambda item: (item["name"], item["version"])
    ):
        references = []
        for source, data in texts_for(package, root, supplements):
            identity = digest(data)
            notices[identity] = data.decode("utf-8")
            references.append({"source": source, "sha256": identity})
        packages.append(
            {
                "name": package["name"],
                "version": package["version"],
                "license": license_of(package),
                "repository": package["repository"],
                "source": package["source"] or "pgorm workspace",
                "notices": references,
            }
        )
    components = [
        {
            "name": "ring native cryptography (including BoringSSL and fiat code)",
            "owner": "ring@0.17.14",
            "license": "see ring notices",
        },
    ]
    report = {
        "schema_version": 1,
        "scope": "Locked Cargo graph, including build and platform-conditional dependencies; a superset of any one wheel",
        "cargo_lock_sha256": digest((root / "pgorm-python/Cargo.lock").read_bytes()),
        "python_runtime_dependencies": [],
        "packages": packages,
        "bundled_native_components": components,
    }
    text = "pgorm Python distribution — third-party notices\n\n"
    text += "The companion DEPENDENCIES.json maps packages and bundled native components to the notice hashes below.\n\n"
    for identity, body in sorted(notices.items()):
        text += f"===== SHA-256 {identity} =====\n{body}\n\n"
    return json.dumps(report, indent=2) + "\n", text


# [spec:pgorm:req:python.distribution]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--write",
        action="store_true",
        help="regenerate notices after dependency changes",
    )
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    manifest, notices = generate(root)
    for name, content in (
        ("DEPENDENCIES.json", manifest),
        ("THIRD_PARTY_NOTICES.txt", notices),
    ):
        path = root / "pgorm-python/python/pgorm" / name
        if options.write:
            path.write_text(content)
        elif not path.is_file() or path.read_bytes() != content.encode("utf-8"):
            raise SystemExit(f"{name} is stale; run checks/notices.py --write")
    print("Verified locked dependency metadata and native component notices.")


if __name__ == "__main__":
    main()
