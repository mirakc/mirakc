SHELL := $(shell which bash) -eu -o pipefail -c

.PHONY: all
all: build

.PHONY: list-targets
list-targets:
	@grep -E '^\.PHONY: ' $(MAKEFILE_LIST) | cut -d ' ' -f 2 | grep -v '^\$$' | sort

.PHONY: check
check: check-rust

# `cargo deny check -s` will fail if a crate having a build script is added.
.PHONY: check-rust
check-rust:
	cargo fmt --all --check
	cargo check --workspace --all-targets --all-features
	cargo clippy --workspace --all-targets --all-features -- -D warnings
	cargo deny check -s

.PHONY: build
build: OPTIONS ?=
build:
	cargo build $(OPTIONS)

# Always run w/ --all-features.
#
# Check slower tests:
#   make test OPTIONS='--status-level=all'.
#
.PHONY: test
test: OPTIONS ?=
test: TESTNAME ?=
test:
	cargo nextest run --all-features $(OPTIONS) $(TESTNAME)

.PHONY: format
format: format-rust

.PHONY: format-rust
format-rust:
	@echo 'Formatting *.rs...'
	@cargo fmt --all

.PHONE: alpine-image
alpine-image: TAG ?= dev
alpine-image: PLATFORM ?= linux/amd64
alpine-image: TARGET ?= mirakc
alpine-image:
	docker buildx build -t mirakc/$(TARGET):$(TAG) -f docker/Dockerfile.alpine --load \
	  --target $(TARGET) --platform=$(PLATFORM) .

.PHONE: debian-image
debian-image: TAG ?= dev
debian-image: PLATFORM ?= linux/amd64
debian-image: TARGET ?= mirakc
debian-image: DEBIAN ?= bookworm
debian-image:
	docker buildx build -t mirakc/$(TARGET):$(TAG) -f docker/Dockerfile.debian --load \
	  --target $(TARGET) --platform=$(PLATFORM) --build-arg DEBIAN_CODENAME=$(DEBIAN) .
