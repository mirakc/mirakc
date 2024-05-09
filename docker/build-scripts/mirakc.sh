set -eu

BASEDIR=$(cd $(dirname $0); pwd)
DISTRO=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3
PROFILE=$4

. $BASEDIR/vars.sh

TRIPLE=$(echo "$RUST_TARGET_TRIPLE" | tr '-' '_' | tr [:lower:] [:upper:])

# Enforce to use a specific compiler in the cc crate.
export CC_${TRIPLE}="$GCC"

# Use environment variables instead of creating .cargo/config:
# https://doc.rust-lang.org/cargo/reference/config.html
# https://github.com/japaric/rust-cross#cross-compiling-with-cargo
export CARGO_TARGET_${TRIPLE}_LINKER="$GCC"

export PKG_CONFIG_ALLOW_CROSS=1

cargo build -v --profile=$PROFILE --target $RUST_TARGET_TRIPLE --bin mirakc
cp ./target/$RUST_TARGET_TRIPLE/$PROFILE/mirakc /usr/local/bin/

cargo build -v --profile=$PROFILE --target $RUST_TARGET_TRIPLE --bin mirakc-timeshift-fs
cp ./target/$RUST_TARGET_TRIPLE/$PROFILE/mirakc-timeshift-fs /usr/local/bin/
cat <<EOF >/usr/local/bin/run-mirakc-timeshift-fs
#!/bin/sh
trap 'umount /mnt' EXIT
/usr/local/bin/mirakc-timeshift-fs /mnt
EOF
chmod +x /usr/local/bin/run-mirakc-timeshift-fs
