# Lumpi

Compresses structured logs as small as Zstd-19, packs **100× faster**, and greps the archive **3–10× faster** than searching raw text.

## The problem

Flat JSONL logs repeat keys and enum values millions of times. Lumpi transposes rows into columns, dictionary-encodes repeating values, then compresses with Zstd — the compressor sees a much simpler signal.

## Compression

Numbers from `bash bench.sh` on Apple M3. Zstd-19 baseline uses the same library.

| Dataset | Size | Lumpi ratio | Zstd-19 ratio | Lumpi MB/s | Zstd-19 MB/s |
|---|---|---|---|---|---|
| Nginx access logs | 111 MB | **16.1×** | 13.2× | **221** | 4.2 |
| App logs (level, user_id, path, status) | 53 MB | 13.0× | 13.2× | **236** | 3.5 |
| CloudTrail (UUID request IDs) | 68 MB | 10.1× | 10.1× | **200** | 4.1 |

## Search without decompressing

`lumpi grep` scans the compressed archive directly. Benchmarked against `zstdgrep` (`zstd -d | grep`) and plain `grep` on the raw file, warm cache, Apple M3.

| Query | lumpi ms | zstdgrep ms | grep ms |
|---|---|---|---|
| App logs — `level=ERROR` (99k matches) | **126** | 996 | 440 |
| Nginx — `method=DELETE` (71k matches) | **144** | 2229 | 809 |
| CloudTrail — `eventName=AssumeRole` (20k matches) | **66** | 324 | 502 |
| CloudTrail — `requestID=<uuid>` (1 match, not in dictionary) | **54** | 522 | 505 |

The last row is the skeptic's test: `requestID` is a UUID field that exceeds the cardinality threshold, so it bypasses the string dictionary and is stored raw. Lumpi still wins 10× because the frame layout means it reads 6.7 MB from disk, not 68 MB — regardless of how the field is encoded.

## Install

```bash
cargo install --git https://github.com/Monalar/lumpi
```

Or from source:

```bash
git clone https://github.com/Monalar/lumpi
cd lumpi_compression
cargo install --path .
```

Requires Rust 1.75+. No runtime dependencies.

## Usage

```bash
lumpi pack access.jsonl
lumpi grep access.jsonl.lmp "method=DELETE"
lumpi unpack access.jsonl.lmp
```

`grep` writes matches to stdout and `N matches in Xms` to stderr, so output is pipeable.

## How it works

Lumpi transposes JSONL into columns — all `level` values together, all `status` values together — then encodes each column with a dictionary and compresses with Zstd L9.

High-cardinality fields (UUIDs, request IDs, session tokens) are detected automatically after 100 rows and stored raw, preventing dictionary bloat.

`grep` uses a frame-based layout: each 65 536-row frame has a small scan block (key IDs, types, string IDs) compressed independently from the data block. Grep decompresses only the scan block per frame and skips the data block entirely when the target field is absent.

Nested JSON (GitHub Archive, API dumps) falls back to raw Zstd automatically.

## Scope

| Input | Behavior |
|---|---|
| Flat JSONL (nginx, app logs, CloudTrail, Datadog) | Columnar encoding |
| CSV with header row | Columnar encoding |
| Nested JSON (GitHub Archive, API dumps) | Raw Zstd — detected automatically |

## Reproduce

```bash
bash bench.sh
```

Generates three datasets locally via `python3` and runs `lumpi bench` on them. No downloads required.
