"""Freeze only ZCode usage fields, independently total them, then remove them.

No conversation tables, credentials, or original request identities are copied.
Evidence output contains aggregates and hashes only; keep it in an ignored folder.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import subprocess
import tempfile
import time
from datetime import datetime
from pathlib import Path

FIELDS = (
    "id logical_request_id attempt_index model_id status started_at input_tokens "
    "output_tokens reasoning_tokens cache_creation_input_tokens cache_read_input_tokens "
    "computed_total_tokens provider_total_tokens raw_usage_json"
).split()
RAW_FIELDS = (
    "inputTokens outputTokens reasoningTokens cacheWriteTokens cacheReadTokens totalTokens"
).split()
SCHEMA = """CREATE TABLE model_usage (
    id TEXT PRIMARY KEY, logical_request_id TEXT, attempt_index INTEGER, model_id TEXT,
    status TEXT, started_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
    reasoning_tokens INTEGER, cache_creation_input_tokens INTEGER, cache_read_input_tokens INTEGER,
    computed_total_tokens INTEGER, provider_total_tokens INTEGER, raw_usage_json TEXT);
    CREATE INDEX model_usage_started_model_idx ON model_usage(started_at, model_id);
"""


def masked_identity(value):
    if not isinstance(value, str):
        return value
    if not value.strip():
        return ""
    digest = hashlib.sha256(value.encode("utf-8")).hexdigest()
    return digest if len(value.encode("utf-8")) <= 1024 else digest + "x" * 1025


def counter(value):
    return type(value) is int and 0 <= value <= 1_000_000_000_000


def independent(rows):
    buckets = {}
    skipped = 0
    partial = False
    for values in rows:
        r = dict(zip(FIELDS, values))
        try:
            raw = json.loads(r["raw_usage_json"])
            assert isinstance(raw, dict)
            assert all(isinstance(r[key], str) and r[key].strip()
                       and len(r[key].encode("utf-8")) <= 1024 for key in ["id", "logical_request_id"])
            assert type(r["attempt_index"]) is int and 0 <= r["attempt_index"] <= 100_000
            assert r["status"] in {"completed", "error", "cancelled"}
            assert isinstance(r["model_id"], str) and len(r["model_id"].encode("utf-8")) <= 256
            assert type(r["started_at"]) is int and r["started_at"] > 0
            assert all(counter(r[key]) for key in FIELDS[6:12])
            for key, stored in zip(RAW_FIELDS[:5], [r["input_tokens"], r["output_tokens"],
                    r["reasoning_tokens"], r["cache_creation_input_tokens"], r["cache_read_input_tokens"]]):
                if key not in raw and key not in {"inputTokens", "outputTokens"}:
                    assert stored == 0
                else:
                    assert counter(raw[key]) and raw[key] == stored
            total = r["input_tokens"] + r["output_tokens"]
            assert total == r["computed_total_tokens"]
            if r["provider_total_tokens"] is None:
                assert "totalTokens" not in raw
            else:
                assert counter(r["provider_total_tokens"]) and counter(raw["totalTokens"])
                assert raw["totalTokens"] == total == r["provider_total_tokens"]
            assert r["reasoning_tokens"] <= r["output_tokens"]
            fresh = r["input_tokens"] - r["cache_read_input_tokens"] - r["cache_creation_input_tokens"]
            assert fresh >= 0
            slot = datetime.fromtimestamp((r["started_at"] // 900_000) * 900).astimezone().strftime("%Y-%m-%d %H:%M")
        except (AssertionError, KeyError, TypeError, ValueError, OverflowError, OSError):
            skipped += 1
            partial = True
            continue
        model = r["model_id"].strip()
        if not model:
            model = "unknown"
            partial = True
        if total == 0:
            continue
        tally = buckets.setdefault(slot, {}).setdefault(model,
            {"input": 0, "cache_write": 0, "cache_read": 0, "output": 0})
        tally["input"] += fresh
        tally["cache_write"] += r["cache_creation_input_tokens"]
        tally["cache_read"] += r["cache_read_input_tokens"]
        tally["output"] += r["output_tokens"]
    ordered = {slot: {model: buckets[slot][model] for model in sorted(buckets[slot])} for slot in sorted(buckets)}
    tallies = [value for models in ordered.values() for value in models.values()]
    return {
        "input": sum(t["input"] for t in tallies), "output": sum(t["output"] for t in tallies),
        "cacheRead": sum(t["cache_read"] for t in tallies), "cacheWrite": sum(t["cache_write"] for t in tallies),
        "total": sum(sum(t.values()) for t in tallies), "rowsRead": len(rows), "rowsSkipped": skipped,
        "partial": partial,
        "bucketDigest": hashlib.sha256(json.dumps(ordered, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest(),
    }


def run(args):
    assert 1 <= args.max_rows <= 100_000
    source = args.database.resolve(strict=True)
    if source.is_symlink() or source.stat().st_size > 256 * 1024 * 1024:
        raise ValueError("Source is not a bounded regular database")
    connection = sqlite3.connect(source.as_uri() + "?mode=ro", uri=True, timeout=0.1)
    try:
        connection.execute("PRAGMA query_only=ON")
        connection.execute("PRAGMA cache_size=-1024")
        connection.execute("PRAGMA mmap_size=0")
        connection.setlimit(sqlite3.SQLITE_LIMIT_LENGTH, 64 * 1024)
        deadline = time.monotonic() + 4
        connection.set_progress_handler(lambda: time.monotonic() >= deadline, 1000)
        connection.execute("BEGIN")
        rows = list(connection.execute(
            "SELECT " + ",".join(FIELDS) + " FROM model_usage ORDER BY started_at DESC LIMIT ?",
            (args.max_rows + 1,)))
        sample_only = len(rows) > args.max_rows
        rows = rows[:args.max_rows]
    finally:
        connection.close()
    sanitized = []
    for row in rows:
        row = list(row)
        row[0], row[1] = masked_identity(row[0]), masked_identity(row[1])
        raw = row[-1]
        try:
            assert isinstance(raw, str) and len(raw.encode("utf-8")) <= 4096
            parsed = json.loads(raw)
            assert isinstance(parsed, dict)
            # Preserve only count keys; invalid value types stay invalid without copying their text.
            row[-1] = json.dumps({key: value if type(value) in {int, float, bool, type(None)} else "invalid"
                                  for key, value in parsed.items() if key in RAW_FIELDS},
                                 ensure_ascii=False, separators=(",", ":"))
        except (AssertionError, ValueError, TypeError):
            row[-1] = None
        sanitized.append(tuple(row))
    expected = independent(sanitized)
    with tempfile.TemporaryDirectory(prefix="quotascope-zcode-") as directory:
        temp = Path(directory).resolve()
        assert temp.is_relative_to(Path(tempfile.gettempdir()).resolve())
        frozen = temp / "db.sqlite"
        snapshot = sqlite3.connect(frozen)
        snapshot.executescript(SCHEMA)
        snapshot.executemany("INSERT INTO model_usage VALUES (" + ",".join("?" * len(FIELDS)) + ")", sanitized)
        snapshot.commit()
        snapshot.close()
        digest = hashlib.sha256(frozen.read_bytes()).hexdigest()
        reader = subprocess.run([str(args.reader.resolve(strict=True)), str(frozen)],
                                capture_output=True, text=True, timeout=15, check=True)
        actual = json.loads(reader.stdout)
        differences = [key for key, value in expected.items() if actual.get(key) != value]
        result = {"matched": not differences, "differences": differences, "sampleOnly": sample_only,
                  "scope": "usage-only frozen SQLite snapshot", "copiedConversationTables": False,
                  "originalIdentitiesCopied": False, "frozenSha256": digest,
                  "python": expected, "rust": actual}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"matched": result["matched"], "differences": differences,
                      "sampleOnly": sample_only, "conversationTablesCopied": False}))
    return 0 if result["matched"] else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", type=Path, default=Path.home() / ".zcode/cli/db/db.sqlite")
    parser.add_argument("--reader", type=Path, required=True)
    parser.add_argument("--max-rows", type=int, default=10_000)
    parser.add_argument("--output", type=Path)
    raise SystemExit(run(parser.parse_args()))
