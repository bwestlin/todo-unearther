# Scan-to-report benchmarks

Single-pass smoke measurements on the implementation machine, using optimized
Rust builds and the synthetic fixtures in `benches/reports.rs`. Console uses
100-column wrapping without colors or hyperlinks; enrichment is disabled:

| Fixture | Source findings | Console total | Markdown total | JSON total | HTML total |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10,000 files, sparse | 10 | 10.1 ms | 9.0 ms | 10.2 ms | 11.4 ms |
| 10,000 files, dense | 2,500 | 20.4 ms | 17.8 ms | 16.7 ms | 24.2 ms |
| 100,000 files, sparse | 100 | 45.6 ms | 49.5 ms | 49.1 ms | 47.9 ms |
| 100,000 files, dense | 25,000 | 162.6 ms | 133.1 ms | 120.3 ms | 133.6 ms |
| 100,000 files, half dependencies | 100 | 25.3 ms | 24.1 ms | 25.3 ms | 24.7 ms |

These are single-pass warm-cache smoke results, not general throughput promises.
Fixtures contain tiny source files, and half the files in the dependency case
are excluded before reading. Totals include traversal, extraction, sorting,
fingerprinting, and rendering, but exclude fixture creation, executable startup,
and writing the report to disk. They do not include GitHub or Git attribution.

Reproduce with:

```sh
TODO_BENCH_RUNS=1 cargo bench --locked --bench reports
```

The default is three repetitions at 10k and 100k files. CSV output includes the
separate scan/fingerprint and render timings, candidate counts, findings, and
bytes read. Use `TODO_BENCH_FILES=10000` for a smaller run. Use the CLI's
`--timings` on representative real repositories before choosing worker counts.

The mixed-monorepo case cycles through all seven supported component ecosystems
and puts repeated findings under shared package ancestors. It runs ordinary
file grouping and both flat and tree component layouts on the same fixture.
CSV output includes grouping/layout mode, parsed manifest counts, additional manifest reads, and cached
directory lookups. Manifests read by the scanner should require zero additional
content reads during component assignment. Component timing is included in the
scan/fingerprint column; rendering is still measured separately.

Mixed-monorepo smoke results with TODO-first reports and console wrapping:

| Files | Findings | Grouping | Console | Markdown | JSON | HTML |
| ---: | ---: | --- | ---: | ---: | ---: | ---: |
| 10,000 | 2,500 | File | 20.4 ms | 18.0 ms | 13.6 ms | 17.6 ms |
| 10,000 | 2,500 | Component, flat | 31.4 ms | 34.3 ms | 25.0 ms | 32.5 ms |
| 10,000 | 2,500 | Component, tree | 32.3 ms | 25.3 ms | 20.9 ms | 27.1 ms |
| 100,000 | 25,000 | File | 184.8 ms | 159.5 ms | 120.6 ms | 154.8 ms |
| 100,000 | 25,000 | Component, flat | 279.2 ms | 259.5 ms | 221.1 ms | 240.1 ms |
| 100,000 | 25,000 | Component, tree | 321.9 ms | 232.6 ms | 205.2 ms | 241.2 ms |

Component mode reused all 10 and 100 scanner-read manifests, respectively,
with zero additional manifest-content reads. It performed 122 and 1,202 cached
ancestor-directory lookups across the seven ecosystems. Ordinary file mode performed no
component metadata work. The same warm-cache, tiny-file, and timing-boundary
caveats above apply; these comparisons are not cold-disk performance claims.

History and API time should be measured separately using `--attribution` and
`--github`; both are off on the default path. The mocked HTTP tests exercise
deduplication, bounded request behavior, and timeout handling without a live
GitHub account.
