# Legacy csvq (oracle)

Unmodified copy of `csvq.sh`, `lib/*.awk`, `bench/gen.awk` (the benchmark
data generator) and `LICENSE` from
[midat-fx/csv-data-quality-cli](https://github.com/midat-fx/csv-data-quality-cli)
at commit `85721ada96e4bc39bec66e5ec7fc1f64c2564392`.

It is vendored, not a submodule, so that `make test` works from a clean
clone without network access. Nothing here is edited: the differential fuzzer
runs this code as the reference implementation, and `make legacy-check`
verifies the checksums below.

```
6ebfeea232614a0a1056759aabfff9a317cfc6ffd8044c36e2717f0be67f8053  csvq.sh
98aba2d40bbdbcc2a45ad8f666ca176f834e0e26becbc42ad55389fe1d4f8a54  lib/audit.awk
c0b2d71c30c8730e7d11cd13e00007f16d35517f5935e6c200bd0b4de7514a1d  lib/csvlib.awk
93674fca8517864a20aab37d096b73ab243e6ca2da93c2b334a2415c01ae089a  lib/dedupe.awk
3dadc2af27481aacb7f5f33db9528394c096cc4001933b0b035919794c52f216  lib/diff.awk
d7fd3ca83820fcffb00aa32a26478f928775fd44b67d48088f23f625553c105b  lib/report_build.awk
761bfed0189e96e633e13cce963c45dccf34b5afe229b0ea14581c412d59829c  lib/stats.awk
95bd5fa35e3be3702910a2d552df667d568367ba1c55d49acfea482a3aa7d15f  lib/validate.awk
191df3ecd3bde468f29636780815d52769b9ca4c6b1a6d7ec53b631c2e455d15  LICENSE
a9e6471a5a714317f1b6facbf20229023019a4ed152e4d30ed50a02f4c8e8f26  bench/gen.awk
```
