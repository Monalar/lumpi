# Lumpi

Search compressed log archives without decompressing them. Pack flat JSONL and CSV logs into `.lmp` files, then `grep` directly against the archive — faster than grepping the raw text, and often faster than grepping an uncompressed file.

## The problem

Flat JSONL logs repeat keys and enum values millions of times. Lumpi transposes rows into columns, dictionary-encodes repeating values, then compresses with Zstd L9 (multithreaded). The compressor sees a much simpler signal, and the frame layout lets grep skip most of the data entirely.

## Search without decompressing

`lumpi grep` scans the compressed archive directly. Each 65 536-row frame stores a small scan block (key IDs, types, dictionary IDs) separately from the data block. Grep decompresses only the scan block and skips the data block when the target field is absent — typically reading less than 10% of the file.

Benchmarked against `zstdgrep` (`zstd -d | grep`, L19-compressed baseline) and plain `grep` on raw text. Warm cache, Apple M3.

| Query | matches | lumpi ms | zstdgrep ms | grep ms |
|---|---|---|---|---|
| App logs — `level=ERROR` | 99 734 | **128** | 447 | 426 |
| Nginx — `method=DELETE` | 71 571 | **146** | 831 | 805 |
| CloudTrail — `eventName=CreateRole` | 19 948 | **68** | 268 | 246 |
| CloudTrail — `requestID=<uuid>` (not in dictionary) | 1 | **56** | 312 | 291 |

The UUID row is the skeptic's test: `requestID` exceeds the cardinality threshold and is stored raw, bypassing the dictionary entirely. Lumpi still wins because the frame layout limits how much data it reads from disk, regardless of how the field is encoded.

## Compression

**The thesis:** columnar transposition lets Zstd L9 multithreaded reach Zstd L19+LDM compression ratios — while being ~110× faster. On app logs (53 MB, Apple M3, 11 threads): lumpi 232 MB/s vs Zstd L19+LDM 2.1 MB/s, with essentially equal ratio (12.99× vs 13.04×). On nginx access logs, columnar encoding exceeds L19+LDM by 20% (15.7× vs 13.0×). Plain Zstd L9 MT alone does not reach L19 — the columnar step is what closes the gap.

Numbers from `bash bench.sh`, Apple M3, 11 threads.

| Dataset | Size | Lumpi ratio | Zstd L9 MT ratio | Zstd L19+LDM ratio | Lumpi MB/s | Zstd L9 MT MB/s |
|---|---|---|---|---|---|---|
| App logs (level, user\_id, path, status) | 53 MB | **12.99×** | 10.56× | 13.04× | 239 | 388 |
| CloudTrail (UUID request IDs) | 68 MB | **10.00×** | 8.44× | 10.11× | 204 | 482 |
| Nginx access logs | 111 MB | **15.68×** | 10.63× | 13.01× | 225 | 561 |

Zstd L19+LDM throughput: ~2 MB/s (single-threaded, same machine). Run `lumpi research <file>` for a full per-file breakdown including Brotli and Gzip.

## Round-trip semantics

Lumpi is a **canonical archive format for logs**, not a byte-exact compressor. `pack → unpack` produces semantically equivalent records, not byte-identical output:

- JSON formatting changes: keys are separated by `", "` and objects end with `\n`
- Integer normalization: integers parsed from strings round-trip as integers
- Whitespace is not preserved
- CSV input unpacks as JSONL (one JSON object per row, column names as keys)
- `null` values are not currently preserved (the field is omitted)

If you need byte-exact storage, lumpi automatically falls back to raw Zstd for files it cannot parse as flat JSONL or CSV. That path is byte-exact and verified with SHA-256 on unpack.

## Install

```bash
brew install Monalar/tap/lumpi
```

Or with Cargo:

```bash
cargo install lumpi
```

Or from source:

```bash
git clone https://github.com/Monalar/lumpi
cd lumpi
cargo install --path .
```

Cargo requires Rust stable. No runtime dependencies.

## Usage

```bash
lumpi pack access.jsonl
lumpi grep access.jsonl.lmp "method=DELETE"
lumpi unpack access.jsonl.lmp
```

`grep` writes matches to stdout and `N matches in Xms` to stderr, so output is pipeable:

```bash
lumpi grep access.jsonl.lmp "status=500" | jq .
```

## How it works

Lumpi transposes JSONL into columns — all `level` values together, all `status` values together — then dictionary-encodes each column and compresses with Zstd L9 MT. High-cardinality fields (UUIDs, request IDs, timestamps) are detected automatically after 100 rows and stored as literals rather than dictionary entries, preventing bloat.

The file is divided into 65 536-row frames. Each frame has two independently compressed blocks:

- **Scan block** — key IDs, value types, string dictionary IDs. Small (~1–3% of frame size).
- **Data block** — varint-encoded integers, literal values.

`grep` decompresses only scan blocks. If the target key is absent from a frame, or a dictionary-encoded string field has no matching ID, the data block is skipped entirely.

Nested JSON (GitHub Archive, API dumps) falls back to raw Zstd automatically.

## Scope

| Input | Behavior |
|---|---|
| Flat JSONL (nginx, app logs, CloudTrail, Datadog) | Columnar encoding |
| CSV with header row | Columnar encoding → JSONL on unpack |
| Nested JSON (GitHub Archive, API dumps) | Raw Zstd, byte-exact, SHA-256 verified |

## Reproduce

```bash
bash bench.sh
```

Requires `python3` and `lumpi` in PATH. Generates three datasets locally and runs the full benchmark suite. No downloads required.
