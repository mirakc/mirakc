.PHONY: all
all: build

.PHONY: build
build: format
	cargo build --all-features

.PHONY: test
test: build
	cargo nextest run --all-features

.PHONY: format
format:
	cargo fmt
