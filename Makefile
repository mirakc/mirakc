SHELL := $(shell which bash) -eu -o pipefail -c

DEBIAN ?= trixie

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
	cargo clippy --workspace --all-targets --all-features -- -D warnings -A 'clippy::collapsible_if'
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

.PHONE: images
images: PREFIX ?= dev
images: DISTRO ?= debian
images: PLATFORM ?= linux/amd64
images:
	$(MAKE) -s image PREFIX=$(PREFIX) DISTRO=$(DISTRO) PLATFORM=$(PLATFORM) TARGET=mirakc
	$(MAKE) -s image PREFIX=$(PREFIX) DISTRO=$(DISTRO) PLATFORM=$(PLATFORM) TARGET=timeshift-fs

.PHONE: image
image: PREFIX ?= dev
image: DISTRO ?= debian
image: PLATFORM ?= linux/amd64
image: TARGET ?= mirakc
image:
	docker build -t mirakc/$(TARGET):$(PREFIX)-$(DISTRO) -f docker/Dockerfile --load \
	  --target $(TARGET)-$(DISTRO) --platform=$(PLATFORM) --build-arg DEBIAN_CODENAME=$(DEBIAN) .

.PHONE: check-all-images
check-all-images: PREFIX ?= dev
check-all-images:
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=debian PLATFORM=linux/386
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=debian PLATFORM=linux/amd64
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=debian PLATFORM=linux/arm/v5
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=debian PLATFORM=linux/arm/v7
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=debian PLATFORM=linux/arm64
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=linux/386
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=linux/amd64
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=linux/arm/v7
	$(MAKE) -s check-images PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=linux/arm64

.PHONE: check-images
check-images: PREFIX ?= dev
check-images: DISTRO ?= debian
check-images: PLATFORM ?= linux/amd64
check-images:
	$(MAKE) -s images PREFIX=$(PREFIX) DISTRO=$(DISTRO) PLATFORM=$(PLATFORM)
	$(MAKE) -s check-mirakc-image PREFIX=$(PREFIX) DISTRO=$(DISTRO) PLATFORM=$(PLATFORM)
	$(MAKE) -s check-timeshift-fs-image PREFIX=$(PREFIX) DISTRO=$(DISTRO) PLATFORM=$(PLATFORM)

.PHONE: check-mirakc-image
check-mirakc-image: PREFIX ?= dev
check-mirakc-iamge: DISTRO ?= debian
check-mirakc-image: PLATFORM ?= linux/amd64
check-mirakc-image:
	docker run --rm --platform=$(PLATFORM) mirakc/mirakc:$(PREFIX)-$(DISTRO) --version
	docker run --rm --platform=$(PLATFORM) --entrypoint=recdvb mirakc/mirakc:$(PREFIX)-$(DISTRO) --version
	docker run --rm --platform=$(PLATFORM) --entrypoint=recpt1 mirakc/mirakc:$(PREFIX)-$(DISTRO) --version
	docker run --rm --platform=$(PLATFORM) --entrypoint=mirakc-arib mirakc/mirakc:$(PREFIX)-$(DISTRO) --version
	docker run --rm --platform=$(PLATFORM) --entrypoint=dvbv5-zap mirakc/mirakc:$(PREFIX)-$(DISTRO) --version

.PHONE: check-timeshift-fs-image
check-timeshift-fs-image: PREFIX ?= dev
check-timeshift-fs-image: DISTRO ?= debian
check-timeshift-fs-image: PLATFORM ?= linux/amd64
check-timeshift-fs-image:
	docker run --rm --platform=$(PLATFORM) --entrypoint=mirakc-timeshift-fs mirakc/timeshift-fs:$(PREFIX)-$(DISTRO) --version

.PHONE: debian-images
debian-images: PREFIX ?= dev
debian-images: PLATFORM ?= linux/amd64
debian-images:
	$(MAKE) -s images PREFIX=$(PREFIX) DISTRO=debian PLATFORM=$(PLATFORM)

.PHONE: alpine-images
alpine-images: PREFIX ?= dev
alpine-images: PLATFORM ?= linux/amd64
alpine-images:
	$(MAKE) -s images PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=$(PLATFORM)

.PHONE: debian-image
debian-image: PREFIX ?= dev
debian-image: PLATFORM ?= linux/amd64
debian-image: TARGET ?= mirakc
debian-image:
	$(MAKE) -s image PREFIX=$(PREFIX) DISTRO=debian PLATFORM=$(PLATFORM) TARGET=$(TARGET)

.PHONE: alpine-image
alpine-image: PREFIX ?= dev
alpine-image: PLATFORM ?= linux/amd64
alpine-image: TARGET ?= mirakc
alpine-image:
	$(MAKE) -s image PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=$(PLATFORM) TARGET=$(TARGET)

.PHONY: update-deps
update-deps: update-deps-crates update-deps-deno

# Specify `CARGO_REGISTRIES_CRATES_IO_PROTOCOL=git` if `make update-deps-crates` gets stuck.
# Perform `cargo update` after `cargo upgrade` in order to update `Cargo.lock`.
.PHONY: update-deps-crates
update-deps-crates:
	cargo upgrade -i allow
	cargo update
