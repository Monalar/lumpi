# Lumpi

Search compressed log archives without decompressing them. Pack flat JSONL and CSV logs into `.lmp` files, then `grep` directly against the archive — faster than grepping the raw text, and often faster than grepping an uncompressed file.

## The problem

Flat JSONL logs repeat keys and enum values millions of times. Lumpi transposes rows into columns, dictionary-encodes repeating values, then compresses with Zstd L9 (multithreaded). The compressor sees a much simpler signal, and the frame layout lets grep skip most of the data entirely.

## Search without decompressing

`lumpi grep` scans the compressed archive directly. Each 65 536-row frame stores a small scan block (key IDs, types, dictionary IDs) separately from the data block. Grep decompresses only the scan block and skips the data block when the target field is absent — typically reading less than 10% of the file.

Benchmarked against `zstdgrep` (`zstd -d | grep`, L19-compressed baseline) and plain `grep` on raw text. Warm cache, Apple M3 Pro.

| Query | matches | lumpi ms | zstdgrep ms | grep ms |
|---|---|---|---|---|
| App logs — `level=ERROR` | 100 139 | **150** | 478 | 443 |
| Nginx — `method=DELETE` | 71 211 | **153** | 876 | 831 |
| CloudTrail — `eventName=CreateRole` | 19 845 | **71** | 272 | 252 |
| CloudTrail — `requestID=<uuid>` (not in dictionary) | 1 | **60** | 557 | 541 |

The UUID row is the skeptic's test: `requestID` exceeds the cardinality threshold and is stored raw, bypassing the dictionary entirely. Lumpi still wins because the frame layout limits how much data it reads from disk, regardless of how the field is encoded.

## Compression

Lumpi transposes JSONL/CSV logs into columns before compressing, so Zstd L9 reaches the compression ratio of single-threaded Zstd L19+LDM — on app logs 12.99× vs 13.06×, on nginx 15.69× vs 13.01×. Because L9 is multithreaded and L19+LDM is not, the throughput gap is large: on the 53 MB app-log set lumpi packs at ~210 MB/s vs ~2 MB/s for L19+LDM (Apple M3, single run — rerun bench.sh on your hardware). Plain Zstd L9 alone does not reach L19; the columnar step closes the gap.

Numbers from `bash bench.sh`, Apple M3 Pro, 11 threads.

| Dataset | Size | Lumpi ratio | Zstd L9 MT ratio | Zstd L19+LDM ratio | Lumpi MB/s | Zstd L9 MT MB/s |
|---|---|---|---|---|---|---|
| App logs (level, user\_id, path, status) | 53 MB | **12.99×** | 10.56× | 13.06× | 214 | 301 |
| CloudTrail (UUID request IDs) | 68 MB | **10.00×** | 8.44× | 10.11× | 193 | 432 |
| Nginx access logs | 111 MB | **15.69×** | 10.63× | 13.01× | 225 | 497 |

Zstd L19+LDM throughput: ~2 MB/s (single-threaded, same machine). Run `lumpi research <file>` for a full per-file breakdown including Brotli and Gzip.

## Round-trip semantics

Lumpi is a **canonical archive format for logs**, not a byte-exact compressor. `pack → unpack` produces semantically equivalent records, not byte-identical output:

- JSON formatting changes: keys are separated by `", "` and objects end with `\n`
- Integer normalization: integers parsed from strings round-trip as integers
- Whitespace is not preserved
- CSV input unpacks as JSONL (one JSON object per row, column names as keys)
- `null` values are preserved as JSON `null` literals

If you need byte-exact storage, lumpi automatically falls back to raw Zstd for files it cannot parse as flat JSONL or CSV. That path is byte-exact and verified with SHA-256 on unpack.

## Install

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

Homebrew (`brew install Monalar/tap/lumpi`) will be available after the first tagged release.

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
