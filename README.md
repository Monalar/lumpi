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
cargo install --git [https://github.com/nickzozulya/lumpi](https://github.com/nickzozulya/lumpi)