#!/usr/bin/env python3
"""Run bounded fuzzing without publishing inputs from SECURITY.md findings."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import resource
import secrets
import shutil
import signal
import subprocess

ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN = "nightly-2026-10-27"
TARGETS = ("html", "css", "url", "xml", "png", "gif", "deflate", "font", "json", "encoding", "unicode", "page", "js", "js-runtime", "js-execute")
WATCHDOG_GRACE = 120
STAT = re.compile(r"^stat::(number_of_executed_units|average_exec_per_sec|peak_rss_mb):\s*(\d+)\s*$")


def positive(value):
    number = int(value)
    if not 1 <= number <= 3600:
        raise argparse.ArgumentTypeError("must be between 1 and 3600")
    return number


def seed_value(value):
    number = int(value)
    if not 1 <= number <= 0xFFFF_FFFF:
        raise argparse.ArgumentTypeError("seed must be between 1 and 4294967295")
    return number


def statistics(log):
    """Only numeric allowlisted counters may leave the private raw log."""
    values = {}
    with log.open(errors="replace") as stream:
        for line in stream:
            match = STAT.fullmatch(line.strip())
            if match:
                values[match[1]] = int(match[2])
    return values


def run_target(target, seconds, seed=1, root=ROOT):
    # Standalone Unix runner: reproducers must also be private in reused dirs.
    os.umask(0o077)
    fuzz = root / "fuzz"
    lock = fuzz / "Cargo.lock"
    before = hashlib.sha256(lock.read_bytes()).digest()
    for directory in ("corpus", "artifacts", "logs"):
        (fuzz / directory).mkdir(mode=0o700, exist_ok=True)
    corpus = fuzz / "corpus" / target
    corpus.mkdir(mode=0o700, exist_ok=True)
    for source in sorted((fuzz / "seeds" / target).iterdir()):
        destination = corpus / source.name
        if source.is_file() and not destination.exists():
            shutil.copyfile(source, destination)
    artifacts = fuzz / "artifacts" / target
    artifacts.mkdir(mode=0o700, exist_ok=True)
    log = fuzz / "logs" / f"{target}-{secrets.token_hex(8)}.log"
    env = os.environ.copy()
    env["CARGO_NET_OFFLINE"] = "true"
    command = [
        "cargo", f"+{TOOLCHAIN}", "fuzz", "run", target, str(corpus), "--",
        f"-max_total_time={seconds}", "-timeout=10", "-rss_limit_mb=512",
        f"-max_len={65536 if target == 'font' else 4096}", f"-seed={seed}",
        "-print_final_stats=1", f"-artifact_prefix={artifacts}{os.sep}",
    ]
    descriptor = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        # cargo-fuzz has no --locked flag. Check the committed resolution before
        # it builds offline, and reject any lockfile mutation afterward as well.
        metadata = subprocess.run(
            ["cargo", f"+{TOOLCHAIN}", "metadata", "--locked", "--offline",
             "--format-version", "1", "--manifest-path", str(fuzz / "Cargo.toml")],
            cwd=root, env=env, stdout=subprocess.DEVNULL, stderr=output,
            timeout=60, check=False,
        )
        if metadata.returncode:
            print(f"{target}: dependency check failed; private log: {log}")
            return metadata.returncode
        with subprocess.Popen(
            command, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT,
            start_new_session=True,
            preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_CORE, (0, 0)),
        ) as process:
            try:
                result = process.wait(timeout=seconds + WATCHDOG_GRACE)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                result = 124
            except KeyboardInterrupt:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                result = 130
    counts = statistics(log)
    executed = counts.get("number_of_executed_units", 0)
    unchanged = hashlib.sha256(lock.read_bytes()).digest() == before
    success = result == 0 and executed > 0 and unchanged
    print(f"{target}: {'passed' if success else 'FAILED'}; seed={seed}; exit={result}; executions={executed}")
    for name in ("average_exec_per_sec", "peak_rss_mb"):
        if name in counts:
            print(f"{name}={counts[name]}")
    if not unchanged:
        print("ERROR: fuzz/Cargo.lock changed")
    if result == 0 and not executed:
        print("ERROR: no completed fuzz executions reported")
    print(f"Private log: {log}")
    if not success:
        print("Keep logs and artifacts confidential; follow SECURITY.md.")
    return 0 if success else (result if 0 < result < 256 else 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=TARGETS, nargs="?")
    parser.add_argument("--list", action="store_true", help="list implemented targets")
    parser.add_argument("--seconds", type=positive, default=30)
    parser.add_argument("--seed", type=seed_value, default=1)
    args = parser.parse_args()
    if args.list:
        print("\n".join(TARGETS))
        return 0
    if args.target is None:
        parser.error("a target is required (or use --list)")
    return run_target(args.target, args.seconds, args.seed)


if __name__ == "__main__":
    raise SystemExit(main())
