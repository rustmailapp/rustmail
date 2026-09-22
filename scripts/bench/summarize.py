#!/usr/bin/env python3
"""Render results/*.jsonl as Markdown tables (last entry per tool+scenario wins)."""

import argparse
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_RESULTS_DIR = HERE / "results"
ENDPOINTS = ["list_first", "list_deep", "search_rare", "search_common", "get_single"]


def load(results_dir: Path) -> dict[tuple[str, str], dict]:
    latest: dict[tuple[str, str], dict] = {}
    for path in sorted(results_dir.glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            entry = json.loads(line)
            latest[(entry["tool"], entry["scenario"])] = entry
    return latest


def fmt(value) -> str:
    return "–" if value is None else str(value)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results-dir", default=str(DEFAULT_RESULTS_DIR), help="directory of <tool>.jsonl result files")
    args = parser.parse_args()
    rows = load(Path(args.results_dir))
    print("## Ingest\n")
    print("| tool | scenario | msgs | conns | stored/s | accepted/s | time to stored (s) | drain after SMTP (s) | peak mem (MB) | CPU % avg | disk (MB) | rejected / conn errors | complete |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for (tool, scenario), e in sorted(rows.items(), key=lambda kv: (kv[0][1], kv[0][0])):
        i = e["ingest"]
        print(
            f"| {tool} | {scenario} | {e['count']} | {e['conns']} | {i['stored_per_s']} | {i['accepted_per_s']} | "
            f"{i['stored_seconds']} | {i['drain_after_smtp_s']} | {fmt(i.get('peak_rss_mb'))} | {fmt(i.get('cpu_pct_avg'))} | "
            f"{fmt(e.get('disk_mb'))} | {i['rejected']} / {i['conn_errors']} | {i['complete']} |"
        )
    print("\n## HTTP latency (ms, p50 / p95 / p99)\n")
    print("| tool | " + " | ".join(ENDPOINTS) + " | delete-all (s) | disk after delete (MB) |")
    print("|---|" + "---|" * (len(ENDPOINTS) + 2))
    for (tool, scenario), e in sorted(rows.items()):
        if "http" not in e:
            continue
        cells = [f"{e['http'][k]['p50_ms']} / {e['http'][k]['p95_ms']} / {e['http'][k]['p99_ms']}" for k in ENDPOINTS]
        print(f"| {tool} | " + " | ".join(cells) + f" | {e['delete']['delete_all_s']} | {fmt(e.get('disk_mb_after_delete'))} |")
    print("\n## Restart and idle memory\n")
    print("| tool | startup empty (s) | idle mem empty (MB) | mem before restart (MB) | restart → full count (s) | count after restart | idle mem after restart (MB) |")
    print("|---|---|---|---|---|---|---|")
    for (tool, scenario), e in sorted(rows.items()):
        if "restart" not in e:
            continue
        r = e["restart"]
        print(
            f"| {tool} | {e['startup_s']} | {e['idle_rss_empty_mb']} | {r['rss_before_restart_mb']} | {r['restart_to_api_s']} | "
            f"{r['count_after_restart']} | {r['idle_rss_after_restart_mb']} |"
        )


if __name__ == "__main__":
    main()
