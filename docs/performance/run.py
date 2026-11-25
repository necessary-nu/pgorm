#!/usr/bin/env python3
"""Build immutable benchmark artifacts, run matched blocks, summarize evidence.

Uses only the Python standard library. Builds and measurements are separate
commands so the caller can finish other compilation before the timed window.
"""
# [spec:pgorm:req:turso.performance]

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
REFERENCE = ROOT / "docs/performance/postgres-before-turso.json"
NATIVE_MANIFEST = ROOT / "benches/native/Cargo.toml"
DEFAULT_REFERENCE_BINARY = Path(
    "/tmp/pgorm-perf-target/release/deps/driver_baseline-1b2e40337e00962a"
)


def output(command):
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def blob(path):
    return output(["git", "hash-object", str(path)])


def fingerprint():
    names = subprocess.check_output(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"], cwd=ROOT
    ).decode().split("\0")
    names = {name for name in names if name.endswith((".rs", "Cargo.toml", "Cargo.lock"))}
    names.add("Cargo.lock")
    data = bytearray()
    for name in sorted(names):
        path = ROOT / name
        if path.is_file():
            data.extend(name.encode() + b"\0" + path.read_bytes() + b"\0")
    return subprocess.check_output(
        ["git", "hash-object", "--stdin"], cwd=ROOT, input=data
    ).decode().strip()


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def build(args):
    artifact_dir = args.artifacts.resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    reference = json.loads(REFERENCE.read_text())
    if blob(args.reference_binary) != reference["executable_git_blob_hash"]:
        raise RuntimeError("preserved reference binary does not match the original report")
    rustc = output(["rustc", "--version", "--verbose"])
    if rustc != reference["available_rustc_at_run"]:
        raise RuntimeError("compiler differs from preserved reference; use its recorded toolchain")
    if not NATIVE_MANIFEST.with_name("Cargo.lock").exists():
        raise RuntimeError("generate/review benches/native/Cargo.lock before locked builds")
    before = fingerprint()
    reference_path = artifact_dir / "postgres-before"
    shutil.copy2(args.reference_binary, reference_path)
    manifest = {
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "source_root": str(ROOT),
        "source_fingerprint": before,
        "git_head": output(["git", "rev-parse", "HEAD"]),
        "git_status": output(["git", "status", "--short"]),
        "rustc": rustc,
        "cargo": output(["cargo", "--version"]),
        "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "root_lockfile_hash": blob(ROOT / "Cargo.lock"),
        "native_lockfile_hash": blob(NATIVE_MANIFEST.with_name("Cargo.lock")),
        "build_environment": {
            key: value for key, value in os.environ.items()
            if key in ("RUSTC", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTUP_TOOLCHAIN", "CARGO_BUILD_TARGET", "CC", "CXX", "AR", "CFLAGS", "CXXFLAGS")
            or key.startswith(("CARGO_PROFILE_", "CARGO_TARGET_"))
        },
        "artifacts": {
            "postgres-before": {
                "path": str(reference_path),
                "executable_git_blob_hash": blob(reference_path),
                "reference_report": str(REFERENCE.relative_to(ROOT)),
                "reference_report_sha256": hashlib.sha256(REFERENCE.read_bytes()).hexdigest(),
                "original_build_command": "cargo bench --bench driver_baseline --offline --target-dir /tmp/pgorm-perf-target -- --output docs/performance/postgres-before-turso.json",
                "compiled_source_head": reference["git_head_at_run_end"],
                "compiled_source_fingerprint": reference["source_fingerprint_at_run_start"],
                "compiled_features": reference["cargo_features"],
            }
        },
    }
    commands = {
        "postgres-off": ["cargo", "bench", "--bench", "driver_baseline", "--no-run"],
        "postgres-on": ["cargo", "bench", "--bench", "driver_baseline", "--features", "turso", "--no-run"],
        "native": ["cargo", "build", "--manifest-path", str(NATIVE_MANIFEST), "--release"],
        "native-alloc": ["cargo", "build", "--manifest-path", str(NATIVE_MANIFEST), "--release", "--features", "alloc-profile"],
    }
    for name, command in commands.items():
        command += ["--locked", "--offline", "--message-format=json", "--target-dir", str(args.target.resolve())]
        print("Building", name, flush=True)
        log_path = artifact_dir / f"{name}-build.jsonl"
        stderr_path = artifact_dir / f"{name}-build.stderr"
        with log_path.open("w") as log, stderr_path.open("w") as errors:
            subprocess.run(command, cwd=ROOT, stdout=log, stderr=errors, check=True)
        executable = None
        expected = "driver_baseline" if name.startswith("postgres") else "native-baseline"
        features = None
        compiled_crates = {}
        for line in log_path.read_text().splitlines():
            event = json.loads(line)
            if event.get("reason") != "compiler-artifact":
                continue
            if event["target"]["name"] in {"pgorm", "pgorm_driver", "pgorm_pool", "turso_core", "turso_pg"}:
                compiled_crates[event["target"]["name"]] = {
                    "package_id": event["package_id"],
                    "features": event["features"],
                    "profile": event["profile"],
                }
            if event["target"]["name"] == expected and event.get("executable"):
                executable = Path(event["executable"])
                features = event["features"]
        if executable is None:
            raise RuntimeError(f"Cargo did not report the {name} executable")
        orm_features = set(compiled_crates["pgorm"]["features"])
        if ("turso" in orm_features) != (name != "postgres-off"):
            raise RuntimeError(f"unexpected native feature state for {name}")
        if name.startswith("native"):
            if orm_features & {"tests-cfg", "metrics-fingerprint"}:
                raise RuntimeError("native runs must use ordinary defaults plus turso")
        else:
            for feature, enabled in reference["cargo_features"].items():
                if feature != "turso" and (feature in orm_features) != enabled:
                    raise RuntimeError(f"{name} changed historical feature {feature}")
        destination = artifact_dir / name
        shutil.copy2(executable, destination)
        manifest["artifacts"][name] = {
            "path": str(destination), "executable_git_blob_hash": blob(destination),
            "build_command": command, "cargo_reported_features": features,
            "compiled_crates": compiled_crates,
            "build_stdout": str(log_path), "build_stderr": str(stderr_path),
        }
    if fingerprint() != before:
        raise RuntimeError("source changed during builds; rebuild after sources freeze")
    write_json(artifact_dir / "build.json", manifest)


def run(args):
    artifact_dir = args.artifacts.resolve()
    manifest = json.loads((artifact_dir / "build.json").read_text())
    if fingerprint() != manifest["source_fingerprint"]:
        raise RuntimeError("source differs from benchmark build; rebuild before measuring")
    for artifact in manifest["artifacts"].values():
        if blob(Path(artifact["path"])) != artifact["executable_git_blob_hash"]:
            raise RuntimeError("benchmark executable changed after build")
    destination = args.output.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    write_json(destination / "build.json", manifest)
    schedule = []
    suites = ["postgres", "native", "alloc", "memory"] if args.suite == "all" else [args.suite]
    for repeat in range(args.repeats):
        if "postgres" in suites:
            names = ["postgres-before", "postgres-off", "postgres-on"]
            names = names[-(repeat % 3):] + names[:-(repeat % 3)] if repeat % 3 else names
            for name in names:
                schedule.append((name, "postgresql", repeat, ["--samples", "1000", "--warmup", "100", "--rounds", "3", "--workers", "4", "--pool-size", "4", "--rows", "128"]))
        for suite in ["native", "alloc", "memory"]:
            if suite not in suites:
                continue
            for database in (["memory", "file"] if repeat % 2 == 0 else ["file", "memory"]):
                binary = "native" if suite == "native" else "native-alloc"
                options = ["--database", database, "--samples", "5000", "--warmup", "500", "--rounds", "3", "--workers", "4", "--pool-size", "4", "--rows", "128"]
                if suite == "memory":
                    options += ["--stream-memory", "true"]
                schedule.append((binary, f"{suite}-{database}", repeat, options))
    for sequence, (name, group, repeat, options) in enumerate(schedule):
        artifact = manifest["artifacts"][name]
        filename = f"{sequence:02}-{group}-{name}-{repeat}.json"
        report_path = destination / filename
        if report_path.exists():
            raise RuntimeError(f"refusing to overwrite existing measurement {report_path}")
        command = [artifact["path"], *options, "--source-root", str(ROOT), "--output", str(report_path)]
        print("Measuring", filename, flush=True)
        with report_path.with_suffix(".stderr").open("w") as log:
            subprocess.run(command, cwd=ROOT, stderr=log, check=True)
        report = json.loads(report_path.read_text())
        if report["source_tree_changed_during_run"] or fingerprint() != manifest["source_fingerprint"]:
            raise RuntimeError("source changed during the measurement window")
        if report["executable_git_blob_hash"] != artifact["executable_git_blob_hash"]:
            raise RuntimeError("measured binary does not match build record")
        if report.get("other_active_server_sessions_at_start", 0):
            raise RuntimeError("PostgreSQL had another active client; repeat in a quiet window")
        report["comparison"] = {
            "sequence": sequence, "repeat": repeat, "group": group,
            "artifact": name, "command": command, "build": artifact,
            "runtime_source_note": "Source/compiler inspection in the report describes the runtime checkout. Compilation provenance is the associated build record; postgres-before retains the original binary and original report provenance.",
        }
        write_json(report_path, report)


def distribution(values):
    mean = statistics.mean(values)
    return {
        "count": len(values), "median": statistics.median(values),
        "mean": mean, "minimum": min(values), "maximum": max(values),
        "stdev": statistics.stdev(values) if len(values) > 1 else 0,
        "coefficient_of_variation": statistics.stdev(values) / mean if len(values) > 1 and mean else 0,
    }


def host_snapshot():
    processes = subprocess.run(
        ["ps", "-Ao", "pid,ppid,pcpu,pmem,etime,state,comm"],
        text=True, capture_output=True, check=False,
    )
    rows = []
    observed = {"cargo", "rustc", "clippy-driver", "nplan", "postgres", "native-baseline", "driver_baseline", "clang", "clang++", "ld", "ld64"}
    if processes.returncode == 0:
        for line in processes.stdout.splitlines()[1:]:
            fields = line.split(None, 6)
            if len(fields) != 7:
                continue
            pid, parent, cpu, memory, elapsed, state, executable = fields
            name = Path(executable).name
            if float(cpu) >= 1.0 or name in observed:
                rows.append({"pid": int(pid), "ppid": int(parent), "cpu_percent": float(cpu), "memory_percent": float(memory), "elapsed": elapsed, "state": state, "executable": name})
        rows.sort(key=lambda row: row["cpu_percent"], reverse=True)
    return {
        "utc": datetime.now(timezone.utc).isoformat(),
        "load_average_1_5_15_minutes": os.getloadavg(),
        "logical_cpu_count": os.cpu_count(),
        "process_snapshot_exit": processes.returncode,
        "process_snapshot_error": processes.stderr.strip() or None,
        "processes": rows,
    }


def compare(args):
    builds = {
        variant: json.loads((directory / "build.json").read_text())
        for variant, directory in [("before", args.before), ("after", args.after)]
    }
    current = fingerprint()
    if current != builds["after"]["source_fingerprint"]:
        raise RuntimeError("source differs from candidate build")
    for binary in ["native", "native-alloc"]:
        before = builds["before"]["artifacts"][binary]
        after = builds["after"]["artifacts"][binary]
        if before["compiled_crates"] != after["compiled_crates"]:
            raise RuntimeError("comparison changed native dependency identities, features or profiles")
        for artifact in [before, after]:
            if blob(Path(artifact["path"])) != artifact["executable_git_blob_hash"]:
                raise RuntimeError("comparison binary differs from its build record")
    for field in ["rustc", "root_lockfile_hash", "native_lockfile_hash", "build_environment"]:
        if builds["before"][field] != builds["after"][field]:
            raise RuntimeError(f"comparison changed {field}")
    destination = args.output.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    if any(destination.glob("*.json")):
        raise RuntimeError("comparison output already contains reports")
    for variant, record in builds.items():
        write_json(destination / f"{variant}-build.json", record)
    schedule = []
    for repeat in range(args.repeats):
        modes = ["memory", "file"] if repeat % 2 == 0 else ["file", "memory"]
        for mode in modes:
            workloads = ["stream"] if args.stream_latency_only else ["scalar", "stream", "memory"]
            for work_index, workload in enumerate(workloads):
                if args.stream_latency_only:
                    binaries = ["native"]
                elif workload == "memory":
                    binaries = ["native-alloc"]
                else:
                    binaries = ["native", "native-alloc"]
                for profile_index, binary in enumerate(binaries):
                    variants = ["before", "after"] if (repeat + work_index + profile_index) % 2 == 0 else ["after", "before"]
                    for variant in variants:
                        options = ["--database", mode, "--samples", "5000", "--warmup", "500", "--rounds", "3", "--workers", "4", "--pool-size", "4", "--rows", "128"]
                        options += ["--stream-memory", "true"] if workload == "memory" else ["--workload", workload]
                        schedule.append((variant, binary, mode, workload, repeat, options))
    window = {
        "command": [sys.executable, *sys.argv],
        "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "runtime_source_fingerprint": current,
        "runtime_head": output(["git", "rev-parse", "HEAD"]),
        "host_platform": platform.platform(),
        "observer_scope": "Start/end snapshots only; intervening activity is not continuously observed. CPU percentages come from ps and are not window utilization. No quiet-host guarantee.",
        "start": host_snapshot(),
    }
    write_json(destination / "window.json", window)
    started = time.monotonic()
    try:
        for sequence, (variant, binary, mode, workload, repeat, options) in enumerate(schedule):
            artifact = builds[variant]["artifacts"][binary]
            path = destination / f"{sequence:02}-{variant}-{binary}-{mode}-{workload}-{repeat}.json"
            command = [artifact["path"], *options, "--source-root", str(ROOT), "--output", str(path)]
            print("Comparing", path.name, flush=True)
            with path.with_suffix(".stderr").open("w") as errors:
                subprocess.run(command, cwd=ROOT, stderr=errors, check=True)
            report = json.loads(path.read_text())
            if report["source_tree_changed_during_run"] or fingerprint() != current:
                raise RuntimeError("source changed during comparison")
            if report["executable_git_blob_hash"] != artifact["executable_git_blob_hash"]:
                raise RuntimeError("measured comparison binary changed")
            report["comparison"] = {
                "sequence": sequence, "repeat": repeat, "group": f"batch-{workload}-{mode}",
                "artifact": f"{variant}-{binary}", "command": command, "build": artifact,
                "compiled_source_fingerprint": builds[variant]["source_fingerprint"],
                "runtime_source_note": "Both executables inspect the candidate runtime checkout. Each variant's separate build record identifies its actual compiled source and executable hash.",
            }
            write_json(path, report)
        window["success"] = True
    finally:
        window["elapsed_seconds"] = time.monotonic() - started
        window["end"] = host_snapshot()
        write_json(destination / "window.json", window)


def summarize(args):
    grouped = {}
    memory = []
    for path in sorted(args.input.glob("*.json")):
        report = json.loads(path.read_text())
        if "comparison" not in report:
            continue
        comparison = report["comparison"]
        for measurement in report["measurements"]:
            key = (comparison["group"], comparison["artifact"], measurement["name"])
            grouped.setdefault(key, []).append((comparison["repeat"], measurement))
        for measurement in report.get("stream_memory_measurements", []):
            memory.append({"file": path.name, "database": report["database"], "artifact": comparison["artifact"], "repeat": comparison["repeat"], **measurement})
    rows = []
    for key, measurements in sorted(grouped.items()):
        repeats = {}
        for repeat, measurement in measurements:
            repeats.setdefault(repeat, []).append(measurement)
        row = {"group": key[0], "artifact": key[1], "workload": key[2]}
        for metric in ["p50_ns", "p95_ns", "p99_ns", "operations_per_second"]:
            row[metric] = distribution([m[metric] for _, m in measurements])
            row[f"process_median_{metric}"] = distribution([
                statistics.median(m[metric] for m in values) for values in repeats.values()
            ])
        allocations = [m for _, m in measurements if m.get("allocation_measurements") is not None]
        if allocations:
            row["allocation_profile"] = {
                field: distribution([m["allocation_measurements"][field] / m["samples"] for m in allocations])
                for field in ["allocations", "reallocations", "requested_allocated_bytes"]
            }
            row["peak_live_requested_bytes_above_start"] = distribution([
                m["allocation_measurements"]["peak_live_requested_bytes_above_start"] for m in allocations
            ])
        rows.append(row)
    ratios = []
    for row in rows:
        if row["group"] != "postgresql" or row["artifact"] == "postgres-before":
            continue
        baseline = next((candidate for candidate in rows if candidate["group"] == "postgresql" and candidate["artifact"] == "postgres-before" and candidate["workload"] == row["workload"]), None)
        if baseline:
            ratios.append({
                "artifact": row["artifact"], "workload": row["workload"],
                "p50_ratio_to_preserved_baseline": row["process_median_p50_ns"]["median"] / baseline["process_median_p50_ns"]["median"],
                "throughput_ratio_to_preserved_baseline": row["process_median_operations_per_second"]["median"] / baseline["process_median_operations_per_second"]["median"],
            })
    write_json(args.output, {"groups": rows, "postgres_ratios": ratios, "stream_memory": memory})
    lines = ["| Group | Artifact/workload | Process p50 median (µs) | Process p50 range (µs) | Ops/s median |", "| --- | --- | ---: | ---: | ---: |"]
    for row in rows:
        if row["artifact"].endswith("alloc"):
            continue
        latency = row["process_median_p50_ns"]
        rate = row["process_median_operations_per_second"]["median"]
        lines.append(f"| {row['group']} | {row['artifact']}/{row['workload']} | {latency['median']/1000:.3f} | {latency['minimum']/1000:.3f}–{latency['maximum']/1000:.3f} | {rate:.0f} |")
    args.output.with_suffix(".md").write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    building = commands.add_parser("build")
    building.add_argument("--artifacts", type=Path, required=True)
    building.add_argument("--target", type=Path, default=Path("/tmp/pgorm-perf-target"))
    building.add_argument("--reference-binary", type=Path, default=DEFAULT_REFERENCE_BINARY)
    building.set_defaults(function=build)
    running = commands.add_parser("run")
    running.add_argument("--artifacts", type=Path, required=True)
    running.add_argument("--output", type=Path, required=True)
    running.add_argument("--suite", choices=["all", "postgres", "native", "alloc", "memory"], default="all")
    running.add_argument("--repeats", type=int, default=3)
    running.set_defaults(function=run)
    comparing = commands.add_parser("compare")
    comparing.add_argument("--before", type=Path, required=True)
    comparing.add_argument("--after", type=Path, required=True)
    comparing.add_argument("--output", type=Path, required=True)
    comparing.add_argument("--repeats", type=int, default=3)
    comparing.add_argument("--stream-latency-only", action="store_true")
    comparing.set_defaults(function=compare)
    summarizing = commands.add_parser("summarize")
    summarizing.add_argument("--input", type=Path, required=True)
    summarizing.add_argument("--output", type=Path, required=True)
    summarizing.set_defaults(function=summarize)
    args = parser.parse_args()
    if getattr(args, "repeats", 1) <= 0:
        parser.error("repeats must be positive")
    args.function(args)


if __name__ == "__main__":
    main()
