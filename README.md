## LUMPRESS 6.0 | CLI

```bash
lumpi pack data.jsonl
lumpi pack data.jsonl -o archive.lumpi

lumpi unpack archive.lumpi
lumpi unpack archive.lumpi -o data.jsonl

lumpi research data.jsonl

lumpi bench datasets/
```

Supported formats: JSONL, JSON (arrays), CSV. Unrecognized formats fall back to raw Zstd.

File signature: `LUMP` + 2-byte version (v6.0) at the start of every `.lumpi` file.


## LUMPRESS 6.0 | ULTIMATE ANALYSIS (500 MB JSONL)

| Algorithm          | Size (KB)    | Ratio    | Pack ms      | MB/s      |
| ------------------ | ------------ | -------- | ------------ | --------- |
| GZIP (v6)          |   34783.73   |   14.72x |    2748.06   |    181.95 |
| Zstd (L3)          |   47980.98   |   10.67x |     478.80   |   1044.28 |
| Zstd (L6)          |   37030.39   |   13.83x |    1849.07   |    270.41 |
| Zstd (L9)          |   33872.81   |   15.12x |    3075.78   |    162.56 |
| Zstd (L15)         |   28733.97   |   17.82x |   21172.34   |     23.62 |
| Zstd (L19)         |   26789.45   |   19.11x |  223955.01   |      2.23 |
| Brotli (L3)        |   60041.64   |    8.53x |    1521.22   |    328.68 |
| Brotli (L11)       |   26584.17   |   19.26x |  702103.02   |      0.71 |
| **LUMPRESS (L9)**  |   25532.24   |   20.05x |    2630.12   |    190.11 |

WEISSMAN SCORE (vs Zstd L3): **1.473**


## LUMPRESS 6.0 | SPECTRUM BENCHMARK

| Dataset | Format | Size(MB) | Entropy | Bucket | LUMPI Ratio | Zstd L3 | Weissman | Lumpi (med) | Zstd L3 (med) |
| ------- | ------ | -------- | ------- | ------ | ----------- | ------- | -------- | ----------- | ------------- |
| 01_random_noise.bin  | Raw    |    10.00 |    8.00 | High (Noise)  |       1.00x |       1.00x |    0.506 |     25.17 ms |      4.22 ms |
| 02_real_text.txt     | Raw    |     4.33 |    4.67 | Low           |      87.49x |      82.12x |    0.289 |     13.44 ms |      1.06 ms |
| 03_complex_api.jsonl | JSONL  |     6.83 |    5.14 | Medium        |      46.28x |      42.77x |    0.409 |     28.41 ms |      2.59 ms |
| 05_array.json        | JSON   |     6.70 |    4.35 | Low           |      18.34x |      12.10x |    0.750 |     49.14 ms |      5.93 ms |
| creditcard.csv       | CSV    |   143.84 |    3.67 | Low           |       2.23x |       2.12x |    0.973 |   1494.69 ms |    848.41 ms |
| test_logs.jsonl      | JSONL  |   500.00 |    5.00 | Low           |      20.05x |      10.67x |    1.518 |   2204.27 ms |    500.38 ms |


## LUMPRESSION 5.4 | ENTROPY SPECTRUM BENCHMARK (MEDIAN & STEADY-STATE)

| Dataset | Size(MB) | Entropy | Bucket | Lumpi Ratio | Zstd L3 | Weissman | Lumpi (med) | Zstd L3 (med) |
| --------| -------- | --------| ------ | ----------- | ------- | -------- | ----------- | ------------- |
| 01_random_noise.bin  |    10.00 |    8.00 | Noise         |       1.00x |       1.00x |    0.494 |     24.55 ms |      3.96 ms |
| 02_real_text.txt     |     4.33 |    4.67 | Natural Text   |      87.50x |      82.12x |    0.274 |     12.74 ms |      0.96 ms |
| 03_complex_api.jsonl |     6.83 |    5.14 | Mixed JSON   |      46.28x |      42.77x |    0.403 |     29.24 ms |      2.56 ms |
| test_logs.jsonl      |   500.00 |    5.00 | Target Logs   |      21.01x |      10.67x |    1.488 |   3463.96 ms |    472.06 ms |


## LUMPRESSION 5.3 | ENTROPY SPECTRUM BENCHMARK (MEDIAN & STEADY-STATE)

| Dataset | Size(MB) | Entropy | Bucket | Lumpi Ratio | Zstd L3 | Weissman | Lumpi (med) | Zstd L3 (med) |
| --------| -------- | --------| ------ | ----------- | ------- | -------- | ----------- | ------------- |
<<<<<<< HEAD
| 01_random_noise.bin  |    10.00 |    8.00 | High (Noise)         |       1.00x |       1.00x |    0.494 |     24.55 ms |      3.96 ms |
| 02_real_text.txt     |     4.33 |    4.67 | Low   |      87.50x |      82.12x |    0.274 |     12.74 ms |      0.96 ms |
| 03_complex_api.jsonl |     6.83 |    5.14 | Medium   |      46.28x |      42.77x |    0.403 |     29.24 ms |      2.56 ms |
| test_logs.jsonl      |   500.00 |    5.00 | Low   |      21.01x |      10.67x |    1.489 |   3624.62 ms |    491.08 ms |
