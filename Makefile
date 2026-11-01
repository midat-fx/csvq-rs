# csvq-rs — Rust port of csvq with a differential-fuzzing equivalence proof.
CARGO ?= cargo
JOBS  ?= 4
SEED  ?= 1
CASES ?= 1500
AWK   ?= /usr/bin/awk

.PHONY: test build fuzz fuzz-smoke fuzz-full fuzz-cross legacy-check bench lint clean

## make test: unit + golden tests, legacy checksum check, and a short fuzz run
test: legacy-check
	$(CARGO) test -j$(JOBS)
	$(MAKE) fuzz-smoke

build:
	$(CARGO) build --release -j$(JOBS)

## Differential fuzzing against the legacy script (needs bash + BWK awk).
fuzz-smoke: build
	./target/release/difffuzz --cases $(CASES) --seed $(SEED) --jobs $(JOBS) --a legacy:$(AWK)

fuzz: fuzz-smoke

fuzz-full: build
	./target/release/difffuzz --cases 50000 --seed 2026 --jobs $(JOBS) --a legacy:$(AWK) \
	  --json results/fuzz/rust-vs-bwk.json | tee results/fuzz/rust-vs-bwk.txt

## Legacy vs legacy on other awks: how portable was the original?
fuzz-cross: build
	-./target/release/difffuzz --cases 5000 --seed 2026 --jobs $(JOBS) --a legacy:$(AWK) --b legacy:gawk \
	  --max-fail 100000 --out fuzz-out/cross-gawk --json results/fuzz/bwk-vs-gawk.json | tee results/fuzz/bwk-vs-gawk.txt
	-./target/release/difffuzz --cases 5000 --seed 2026 --jobs $(JOBS) --a legacy:$(AWK) --b legacy:mawk \
	  --max-fail 100000 --out fuzz-out/cross-mawk --json results/fuzz/bwk-vs-mawk.json | tee results/fuzz/bwk-vs-mawk.txt

legacy-check:
	@cd legacy && sed -n '/^```$$/,/^```$$/p' README.md | grep -v '^```' | shasum -a 256 -c --quiet && echo "legacy/: unmodified"

bench: build
	bench/run.sh

lint:
	$(CARGO) fmt --check
	$(CARGO) clippy -j$(JOBS) --all-targets -- -D warnings

clean:
	$(CARGO) clean
	rm -rf fuzz-out
