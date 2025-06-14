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

.PHONE: check-all-images
check-all-images: PREFIX ?= dev
check-all-images: DEBIAN ?= bookworm
check-all-images:
	$(MAKE) -s check-images PREFIX=$(PREFIX) DEBIAN=$(DEBIAN) PLATFORM=linux/386
	$(MAKE) -s check-images PREFIX=$(PREFIX) DEBIAN=$(DEBIAN) PLATFORM=linux/amd64
	$(MAKE) -s check-images PREFIX=$(PREFIX) DEBIAN=$(DEBIAN) PLATFORM=linux/arm/v7
	$(MAKE) -s check-images PREFIX=$(PREFIX) DEBIAN=$(DEBIAN) PLATFORM=linux/arm64

.PHONE: check-images
check-images: PREFIX ?= dev
check-images: DEBIAN ?= bookworm
check-images: PLATFORM ?= linux/amd64
check-images:
	$(MAKE) -s debian-images PREFIX=$(PREFIX) DEBIAN=$(DEBIAN) PLATFORM=$(PLATFORM)
	$(MAKE) -s check-mirakc-image PREFIX=$(PREFIX) DISTRO=debian PLATFORM=$(PLATFORM)
	$(MAKE) -s check-timeshift-fs-image PREFIX=$(PREFIX) DISTRO=debian PLATFORM=$(PLATFORM)
	$(MAKE) -s alpine-images PREFIX=$(PREFIX) PLATFORM=$(PLATFORM)
	$(MAKE) -s check-mirakc-image PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=$(PLATFORM)
	$(MAKE) -s check-timeshift-fs-image PREFIX=$(PREFIX) DISTRO=alpine PLATFORM=$(PLATFORM)

.PHONE: check-mirakc-image
check-mirakc-image: PREFIX ?= dev
check-mirakc-image: DISTRO ?= debian
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
debian-images: DEBIAN ?= bookworm
debian-images:
	$(MAKE) -s debian-image PREFIX=$(PREFIX) PLATFORM=$(PLATFORM) DEBIAN=$(DEBIAN) TARGET=mirakc
	$(MAKE) -s debian-image PREFIX=$(PREFIX) PLATFORM=$(PLATFORM) DEBIAN=$(DEBIAN) TARGET=timeshift-fs

.PHONE: alpine-images
alpine-images: PREFIX ?= dev
alpine-images: PLATFORM ?= linux/amd64
alpine-images:
	$(MAKE) -s alpine-image PREFIX=$(PREFIX) PLATFORM=$(PLATFORM) TARGET=mirakc
	$(MAKE) -s alpine-image PREFIX=$(PREFIX) PLATFORM=$(PLATFORM) TARGET=timeshift-fs

.PHONE: debian-image
debian-image: PREFIX ?= dev
debian-image: PLATFORM ?= linux/amd64
debian-image: TARGET ?= mirakc
debian-image: DEBIAN ?= bookworm
debian-image:
	docker build -t mirakc/$(TARGET):$(PREFIX)-debian -f docker/Dockerfile.debian --load \
	  --target $(TARGET) --platform=$(PLATFORM) --build-arg DEBIAN_CODENAME=$(DEBIAN) .

.PHONE: alpine-image
alpine-image: PREFIX ?= dev
alpine-image: PLATFORM ?= linux/amd64
alpine-image: TARGET ?= mirakc
alpine-image:
	docker run --rm --platform=$(PLATFORM) \
	  --mount="type=bind,src=$(shell pwd)/docker/build-scripts/archive.sh,dst=/archive.sh" \
	  --entrypoint=sh mirakc/$(TARGET):$(PREFIX)-debian /archive.sh >archive.tar.gz
	docker build -t mirakc/$(TARGET):$(PREFIX)-alpine -f docker/Dockerfile.alpine --load \
	  --target $(TARGET) --platform=$(PLATFORM) .
	rm -f archive.tar.gz
