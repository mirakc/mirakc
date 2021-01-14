set -eu

BASEDIR=$(cd $(dirname $0); pwd)
TARGET=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3

. $BASEDIR/vars.sh

TRIPLE=$(echo "$RUST_TARGET_TRIPLE" | tr '-' '_' | tr [:lower:] [:upper:])

# Enforce to use a specific compiler in the cc crate.
export CC="$GCC"

# A workaround to fix the following issue:
# https://github.com/rust-lang/backtrace-rs/issues/249
if [ "$TARGETPLATFORM" = linux/arm/v7 ]; then
  export CFLAGS='-mfpu=neon'
fi

# Use environment variables instead of creating .cargo/config:
# https://doc.rust-lang.org/cargo/reference/config.html
# https://github.com/japaric/rust-cross#cross-compiling-with-cargo
export CARGO_TARGET_${TRIPLE}_LINKER="$GCC"

# A workaround to fix the following issue:
# https://github.com/rust-lang/compiler-builtins/issues/201
if [ "$TARGETPLATFORM" = linux/arm64/v8 ] || [ "$TARGETPLATFORM" = linux/arm64 ]; then
  export CARGO_TARGET_${TRIPLE}_RUSTFLAGS='-C link-arg=-lgcc'
fi

cargo build -v --release --target $RUST_TARGET_TRIPLE
cp /build/target/$RUST_TARGET_TRIPLE/release/mirakc /usr/local/bin/
