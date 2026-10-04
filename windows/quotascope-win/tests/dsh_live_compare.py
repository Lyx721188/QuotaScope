"""Read-only comparison of frozen local DSH bytes, with Python 3.14 stdlib zstd.
Only aggregate results are printed; the temporary private corpus is removed.
Run: python dsh_live_compare.py --reader target/debug/examples/dsh-read.exe
"""
import argparse
import compression.zstd
import hashlib
import io
import json
import pathlib
import subprocess
import tempfile
import time


def counter(value):
    return value if isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= 10**12 else None


def decoded(data):
    if data[:4] == bytes.fromhex("28b52ffd"):
        # The independent decoder accepts every concatenated frame too.
        with compression.zstd.open(io.BytesIO(data), "rb") as stream:
            data = stream.read(16 * 1024 * 1024 + 1)
    if len(data) > 16 * 1024 * 1024:
        raise ValueError("Sample exceeds the comparison ceiling")
    return data.decode("utf-8")


def independent(texts):
    records = {}
    partial = False
    assistant_events = 0
    gaps = {"invalidJson": 0, "missingUsage": 0, "invalidCounters": 0, "missingModelOrTimestamp": 0}
    for text in texts:
        seed, header_model = 0, None
        for line in text.splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                partial = True
                gaps["invalidJson"] += 1
                continue
            kind = row.get("type")
            data = row.get("data") or {}
            if kind == "session":
                seed = counter(row.get("seedLength")) or 0
            elif kind == "request/header":
                candidate = ((data.get("header") or {}).get("config") or {}).get("model")
                if isinstance(candidate, str):
                    header_model = candidate
            elif kind == "assistant/message":
                if (counter(row.get("seq")) or 0) < seed:
                    continue
                assistant_events += 1
                message = data.get("message") or {}
                source = message.get("source") or {}
                response = (source.get("replayState") or {}).get("response") or {}
                candidates = [response.get("responseModel"), source.get("model"), header_model]
                model = next((v for v in candidates if isinstance(v, str)), None)
                at = row.get("time")
                if model is None or not isinstance(at, int) or at <= 0:
                    partial = True
                    gaps["missingModelOrTimestamp"] += 1
                    continue
                usage = data.get("usage")
                if not isinstance(usage, dict):
                    partial = True
                    gaps["missingUsage"] += 1
                    continue
                values = tuple(counter(usage.get(k)) if k in usage else (None if k in ("inputTokens", "outputTokens") else 0)
                               for k in ("inputTokens", "outputTokens", "cacheReadTokens", "cacheWriteTokens"))
                if any(v is None for v in values):
                    partial = True
                    gaps["invalidCounters"] += 1
                    continue
                if sum(values) <= 0:
                    continue
                identity = message.get("id")
                if not isinstance(identity, str):
                    identity = data.get("compactionId")
                if not isinstance(identity, str):
                    identity = f"seq:{counter(row.get('seq')) or 0}"
                records[(identity, at, model, values)] = values
    totals = [sum(value[i] for value in records.values()) for i in range(4)]
    result = dict(zip(("input", "output", "cacheRead", "cacheWrite"), totals))
    result.update(total=sum(totals), partial=partial)
    return result, assistant_events, len(records), gaps


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--reader", required=True, type=pathlib.Path)
    parser.add_argument("--source", type=pathlib.Path, default=pathlib.Path.home() / ".dsh/sessions")
    parser.add_argument("--limit", type=int, default=12)
    args = parser.parse_args()
    if not 1 <= args.limit <= 32:
        parser.error("limit must be 1..32")
    candidates = []
    for path in args.source.rglob("session*"):
        if path.is_file() and not path.is_symlink() and (path.name.endswith(".jsonl") or path.name.endswith(".zstd")):
            stat = path.stat()
            if 0 < stat.st_size <= 512 * 1024 and time.time() - stat.st_mtime >= 60:
                candidates.append((stat.st_mtime, path))
    candidates.sort(reverse=True)
    texts, digests, compressed, skipped = [], [], 0, 0
    with tempfile.TemporaryDirectory(prefix="qs-dsh-compare-") as directory:
        root = pathlib.Path(directory).resolve()
        if root.parent != pathlib.Path(tempfile.gettempdir()).resolve():
            raise RuntimeError("Unexpected temporary corpus location")
        for _, path in candidates:
            if len(texts) >= args.limit:
                break
            before = path.stat()
            data = path.read_bytes()
            after = path.stat()
            if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
                skipped += 1
                continue
            try:
                text = decoded(data)
            except (ValueError, UnicodeError, EOFError, compression.zstd.ZstdError):
                skipped += 1
                continue
            folder = root / str(len(texts))
            folder.mkdir()
            (folder / path.name).write_bytes(data)
            texts.append(text)
            digests.append(hashlib.sha256(data).hexdigest())
            compressed += int(data[:4] == bytes.fromhex("28b52ffd"))
        if not texts or not compressed:
            raise RuntimeError("No stable, genuinely compressed DSH samples available")
        expected, assistant_events, records, gaps = independent(texts)
        output = subprocess.run([str(args.reader.resolve()), str(root)], capture_output=True, text=True, check=True)
        actual = json.loads(output.stdout)
        matched = all(actual.get(key) == value for key, value in expected.items())
        print(json.dumps({"passed": matched, "sampleFiles": len(texts), "compressedFiles": compressed,
                          "skippedCandidates": skipped, "assistantEventsAfterSeed": assistant_events,
                          "distinctRecords": records, "independent": expected, "rust": actual,
                          "readGaps": gaps,
                          "corpusDigest": hashlib.sha256("".join(digests).encode()).hexdigest(),
                          "readerSha256": hashlib.sha256(args.reader.read_bytes()).hexdigest(),
                          "temporaryCorpusRemovedOnExit": True, "realLocalFiles": True}, ensure_ascii=False))
        if not matched:
            raise RuntimeError("Independent aggregate counts disagree")


if __name__ == "__main__":
    main()
