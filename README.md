# Lumpi

Columnar compression for flat JSONL and CSV logs.

**Same ratio as Zstd-L19. 100× faster to compress.**

---

## The problem

Flat JSONL logs repeat the same keys and enum values in every row. A nginx access log with a million entries contains the word `"method"` a million times, and `"GET"` eight hundred thousand times. Plain Zstd treats the file as a blob and relies on its sliding window to find these repetitions. Lumpi transposes the data into columns first — all methods together, all status codes together, all timestamps together — then encodes each column with a dictionary and compresses the result with Zstd. The compressor sees a much simpler signal.

## Results

Numbers from `bash bench.sh` on Apple M3. Zstd baselines run with the same library at their documented optimal settings.

| Dataset | Size | Lumpi ratio | Lumpi MB/s | Zstd-L19+LDM ratio | Zstd-L19+LDM MB/s |
|---|---|---|---|---|---|
| Nginx access logs (IP, UA, path, status, latency) | 110 MB | **15.6×** | 217 | 13.0× | ~2 |
| Application logs (level, user_id, latency, path) | 53 MB | **13.1×** | 222 | 13.0× | ~2 |
| CloudTrail (UUID request IDs, nullable fields) | 68 MB | **10.0×** | 241 | 10.1× | ~2 |

On every dataset Lumpi matches or exceeds Zstd-L19 on ratio while compressing **100× faster**. The nginx result stands out because low-cardinality repeated fields (method, status, path) respond strongly to columnar dictionary encoding. CloudTrail contains a UUID per event — the high-cardinality field is detected automatically after 100 rows and stored raw, keeping ratio on par with Zstd.

Nested JSON (GitHub Archive events, deeply nested API responses) falls back to raw Zstd automatically. Lumpi is not a general-purpose compressor — it is a specialist for flat structured logs.

## Install

```bash
cargo install --git https://github.com/nickzozulya/lumpi
```

Or from source:

```bash
git clone https://github.com/nickzozulya/lumpi
cd lumpi
cargo install --path .
```

Requires Rust 1.75+. No runtime dependencies.

## Usage

```bash
# Compress
lumpi pack access.log.jsonl              # → access.log.jsonl.lmp
lumpi pack access.log.jsonl out.lmp      # explicit output path

# Decompress
lumpi unpack out.lmp                     # → out
lumpi unpack out.lmp restored.jsonl

# Compare against all baselines on one file
lumpi research access.log.jsonl

# Benchmark a directory of files (skips non-flat files automatically)
lumpi bench ./logs/

# Search without full decompression
lumpi grep out.lmp "status=500"
lumpi grep out.lmp "level=ERROR"
lumpi grep out.lmp "user_id=42"
```

`pack` and `unpack` print ratio and throughput. `grep` writes matches to stdout and `N matches in Xms` to stderr, so output is pipeable.

## How it works

1. **Parse** — a zero-copy FSM walks the JSONL byte-by-byte and extracts key-value pairs without allocating per-field strings. Nested objects or arrays trigger an automatic fallback to raw mode.

2. **Transpose** — fields are routed into separate streams: a key-ID stream, a type stream, a string-ID stream, a ZigZag varint stream, and a literal stream (booleans, nulls, floats).

3. **Encode** — string values are interned into a global dictionary (u32 ID). Fields where unique values exceed 50% of occurrences after 100 samples (UUIDs, request IDs, session tokens) are detected automatically and stored raw — preventing dictionary bloat on mixed-cardinality data.

4. **Compress** — all streams concatenated and compressed with Zstd L9 multithreaded. A SHA-256 of the columnar payload is stored in the header for integrity verification on decompression.

## Scope

| Input | Behavior |
|---|---|
| Flat JSONL (nginx, app logs, CloudTrail, Datadog) | Columnar encoding — full ratio benefit |
| CSV with header row | Columnar encoding |
| JSON array of flat objects | Columnar encoding |
| Nested JSON (GitHub Archive, API dumps) | Raw Zstd — detected automatically |
| Binary / unstructured | Raw Zstd — detected automatically |

## Reproduce the benchmark

```bash
bash bench.sh
```

Generates three datasets locally (no download required) and optionally fetches one hour of GitHub Archive events to demonstrate the nested-JSON fallback. Requires `python3` and `lumpi` in PATH.

## Format

- Magic bytes: `LUMP`
- Version: `0x08 0x00`
- Payload: single Zstd block containing a 64-byte SHA-256 hex digest, a schema dictionary mapping key names to u16 IDs, and columnar data streams
- Extension: `.lmp`

Archives produced by older format versions will be rejected with a clear error. Re-compress with the current binary.
