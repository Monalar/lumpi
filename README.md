## LUMPRESSION 5.3 | ENTROPY SPECTRUM BENCHMARK (MEDIAN & STEADY-STATE)

| Dataset | Size(MB) | Entropy | Bucket | Lumpi Ratio | Zstd L3 | Weissman | Lumpi (med) | Zstd L3 (med) |
| --------| -------- | --------| ------ | ----------- | ------- | -------- | ----------- | ------------- |
| 01_random_noise.bin  |    10.00 |    8.00 | High (Noise)         |       1.00x |       1.00x |    0.494 |     24.55 ms |      3.96 ms |
| 02_real_text.txt     |     4.33 |    4.67 | Low   |      87.50x |      82.12x |    0.274 |     12.74 ms |      0.96 ms |
| 03_complex_api.jsonl |     6.83 |    5.14 | Medium   |      46.28x |      42.77x |    0.403 |     29.24 ms |      2.56 ms |
| test_logs.jsonl      |   500.00 |    5.00 | Low   |      21.01x |      10.67x |    1.489 |   3624.62 ms |    491.08 ms |


## LUMPRESSION 5.4 | ENTROPY SPECTRUM BENCHMARK (MEDIAN & STEADY-STATE)


| Dataset | Size(MB) | Entropy | Bucket | Lumpi Ratio | Zstd L3 | Weissman | Lumpi (med) | Zstd L3 (med) |
| --------| -------- | --------| ------ | ----------- | ------- | -------- | ----------- | ------------- |
| 01_random_noise.bin  |    10.00 |    8.00 | High (Noise)         |       1.00x |       1.00x |    0.495 |     25.10 ms |      4.02 ms |
| 02_real_text.txt     |     4.33 |    4.67 | Low   |      87.50x |      82.12x |    0.277 |                    12.71 ms |      0.98 ms |
| 03_complex_api.jsonl |     6.83 |    5.14 | Medium|      46.28x |      42.77x |    0.410 |     28.26 ms |                  2.60 ms |
| test_logs.jsonl      |   500.00 |    5.00 | Low   |      21.01x |      10.67x |    1.540 |   2612.63 ms |                   470.34 ms |
