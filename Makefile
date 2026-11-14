CARGO ?= cargo
ZIGBUILD ?= cargo zigbuild
PLATFORMS := linux-amd64 linux-arm64
IMAGE ?= camd
CONFIG ?= deploy/config.example.toml
DATA ?= .data

triple.linux-amd64 := x86_64-unknown-linux-musl
triple.linux-arm64 := aarch64-unknown-linux-musl
TRIPLE = $(triple.$(PLATFORM))

export CFLAGS_x86_64_unknown_linux_musl := -DSQLITE_DISABLE_LFS
export CFLAGS_aarch64_unknown_linux_musl := -DSQLITE_DISABLE_LFS

.PHONY: build test rust-test web ui lint fmt release-local platform sums run docker firmware screenshots brand clean

build:
	$(CARGO) build --release --locked
	cp target/release/camd camd

test: lint rust-test web

rust-test:
	$(CARGO) test --locked

web:
	cd tests/web && node --test

ui:
	$(CARGO) build --locked
	CAMD_BIN=target/debug/camd tools/ui-test.sh test

lint:
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets --locked -- -D warnings

fmt:
	$(CARGO) fmt

release-local:
	rm -rf dist
	@for p in $(PLATFORMS); do $(MAKE) --no-print-directory platform PLATFORM=$$p || exit 1; done
	$(MAKE) --no-print-directory sums

platform:
	@test -n "$(TRIPLE)" || { echo "PLATFORM must be one of: $(PLATFORMS)" >&2; exit 2; }
	$(ZIGBUILD) --release --locked --target $(TRIPLE)
	mkdir -p dist
	cp target/$(TRIPLE)/release/camd dist/camd-$(PLATFORM)

sums:
	cp deploy/install.sh deploy/camd.service deploy/config.example.toml dist/
	cd dist && sha256sum camd-* install.sh camd.service config.example.toml > SHA256SUMS

run:
	mkdir -p $(DATA)
	CAMD_CONFIG=$(CONFIG) CAMD_DATA_DIR=$(DATA) CAMD_LISTEN_HOST=127.0.0.1 $(CARGO) run --locked

docker:
	docker build -t $(IMAGE) .

firmware:
	cd firmware && pio run

screenshots:
	$(CARGO) build --locked
	CAMD_BIN=target/debug/camd tools/ui-test.sh screenshots

brand:
	tools/render-brand.sh

clean:
	$(CARGO) clean
	rm -rf dist camd
