# Lumpi

Search compressed log archives without decompressing them. Pack flat JSONL and CSV logs into `.lmp` files, then `grep` directly against the archive — faster than grepping the raw text.

## The problem

Flat JSONL logs repeat keys and enum values millions of times. Lumpi transposes rows into columns, dictionary-encodes repeating values, then compresses with Zstd L9 (multithreaded). The compressor sees a much simpler signal, and the frame layout lets grep skip most of the data entirely.

## Search without decompressing

`lumpi grep` scans the compressed archive directly. Each 65 536-row frame stores a small scan block (key IDs, types, dictionary IDs) separately from the data block. Grep decompresses only the scan block and skips the data block when the target field is absent — typically reading less than 10% of the file.

Benchmarked against `zstdgrep` (`zstd -d | grep`) and plain `grep` on the raw file, warm cache, Apple M3. Numbers below are from the last `bash bench.sh` run; regenerate with that command.

| Query | lumpi ms | zstdgrep ms | grep ms |
|---|---|---|---|
| App logs — `level=ERROR` | _run bench.sh_ | | |
| Nginx — `method=DELETE` | | | |
| CloudTrail — `eventName=CreateRole` | | | |
| CloudTrail — `requestID=<uuid>` (not in dictionary) | | | |

The UUID row is the skeptic's test: `requestID` exceeds the cardinality threshold and is stored raw, bypassing the dictionary entirely. Lumpi still wins because the frame layout limits how much data it reads from disk — regardless of how the field is encoded.

## Compression

Lumpi uses Zstd L9 multithreaded internally. The point of the columnar encoding is not to replace Zstd — it is to let a fast compressor (L9 MT) reach the ratio of a slow one (L19). The table below shows all three: lumpi, the plain Zstd L9 MT baseline it is built on, and Zstd L19+LDM as the upper bound.

Numbers below are placeholders — regenerate with `bash bench.sh`.

| Dataset | Size | Lumpi ratio | Zstd L9 MT ratio | Zstd L19+LDM ratio | Lumpi MB/s | Zstd L9 MT MB/s |
|---|---|---|---|---|---|---|
| Nginx access logs | _run bench.sh_ | | | | | |
| App logs (level, user_id, path, status) | | | | | | |
| CloudTrail (UUID request IDs) | | | | | | |

Run `lumpi research <file>` for a full per-file breakdown including Brotli and GZIP.

## Round-trip semantics

Lumpi is a **canonical archive format for logs**, not a byte-exact compressor. `pack → unpack` produces semantically equivalent records, not byte-identical output:

- JSON formatting changes: keys are separated by `", "` and objects end with `\n`
- Integer normalization: `1.0` stored as a float literal; integers parsed from strings round-trip as integers
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

`grep` writes matches to stdout and `N matches in Xms` to stderr, so output is pipeable.

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
