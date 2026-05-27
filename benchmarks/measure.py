#!/usr/bin/env python3
import argparse
import json
import math
import os
import statistics
import subprocess
import sys
import time


def command_label(command: str) -> str:
    if "_ax" in command or " web ax" in command or command.endswith(" ax"):
        return "Ax"
    if "_c" in command or " web c" in command or command.endswith(" c"):
        return "C"
    if "_rust" in command or " web rust" in command or command.endswith(" rust"):
        return "Rust"
    if "python3 " in command or " web python" in command or command.endswith(" python"):
        return "Python"
    if "node " in command or " web node" in command or command.endswith(" node"):
        return "Node"
    return command


def run_command(command: str) -> None:
    completed = subprocess.run(command, shell=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if completed.returncode != 0:
        raise RuntimeError(f"command failed with exit {completed.returncode}: {command}")


def measure(command: str, warmup: int, runs: int) -> list[float]:
    for _ in range(warmup):
        run_command(command)
    timings = []
    for _ in range(runs):
        started = time.perf_counter()
        run_command(command)
        timings.append(time.perf_counter() - started)
    return timings


def summarize(timings: list[float]) -> dict[str, float]:
    stdev = statistics.stdev(timings) if len(timings) > 1 else 0.0
    return {
        "min_seconds": min(timings),
        "median_seconds": statistics.median(timings),
        "mean_seconds": statistics.mean(timings),
        "stdev_seconds": stdev,
    }


def write_markdown(path: str, group: str, entries: list[dict]) -> None:
    best = min(entry["summary"]["median_seconds"] for entry in entries)
    lines = [
        f"# {group}",
        "",
        "| Runtime | Median | Min | Mean | Stddev | Relative |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for entry in sorted(entries, key=lambda item: item["summary"]["median_seconds"]):
        summary = entry["summary"]
        median = summary["median_seconds"]
        relative = median / best if best > 0 else math.inf
        lines.append(
            f"| {entry['label']} | {median:.6f}s | {summary['min_seconds']:.6f}s | "
            f"{summary['mean_seconds']:.6f}s | {summary['stdev_seconds']:.6f}s | {relative:.2f}x |"
        )
    lines.append("")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--group", required=True)
    parser.add_argument("--slug", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("commands", nargs="+")
    args = parser.parse_args()

    os.makedirs(args.out, exist_ok=True)
    entries = []
    for command in args.commands:
        label = command_label(command)
        timings = measure(command, args.warmup, args.runs)
        entry = {
            "label": label,
            "command": command,
            "timings_seconds": timings,
            "summary": summarize(timings),
        }
        entries.append(entry)

    result = {
        "group": args.group,
        "slug": args.slug,
        "runs": args.runs,
        "warmup": args.warmup,
        "entries": entries,
    }
    json_path = os.path.join(args.out, f"{args.slug}.json")
    md_path = os.path.join(args.out, f"{args.slug}.md")
    with open(json_path, "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2)
        handle.write("\n")
    write_markdown(md_path, args.group, entries)
    print(open(md_path, encoding="utf-8").read())
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"benchmark measurement failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
