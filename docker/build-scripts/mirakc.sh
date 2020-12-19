BASEDIR=$(cd $(dirname $0); pwd)

. $BASEDIR/vars.sh

set -eu

TRIPLE=$(echo "$RUST_TARGET_TRIPLE" | tr '-' '_' | tr [:lower:] [:upper:])

# Enforce to use a specific compiler in the cc crate.
export CC="$GCC"

# Used for a workaround to fix the following issue:
# https://github.com/rust-lang/backtrace-rs/issues/249
if [ -n "$MIRAKC_CFLAGS" ]; then
  export CFLAGS="$MIRAKC_CFLAGS"
fi

# Use environment variables instead of creating .cargo/config:
# https://doc.rust-lang.org/cargo/reference/config.html
# https://github.com/japaric/rust-cross#cross-compiling-with-cargo
export CARGO_TARGET_${TRIPLE}_LINKER="$GCC"

# Used for a workaround to fix the following issue:
# https://github.com/rust-lang/compiler-builtins/issues/201
if [ -n "$MIRAKC_RUSTFLAGS" ]; then
  export CARGO_TARGET_${TRIPLE}_RUSTFLAGS="$MIRAKC_RUSTFLAGS"
fi

cargo build -v --release --target $RUST_TARGET_TRIPLE
cp /build/target/$RUST_TARGET_TRIPLE/release/mirakc /usr/local/bin/
