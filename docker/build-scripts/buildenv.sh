set -eu

BASEDIR=$(cd $(dirname $0); pwd)
TARGET=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3

. $BASEDIR/vars.sh

apt-get update
apt-get install -y --no-install-recommends $BUILD_DEPS

rustup target add $RUST_TARGET_TRIPLE

if [ "$TARGET" = alpine ]; then
  # Use a cross-compiler working with musl@1.2.2 used in alpine:3.13.
  #
  # * https://musl.cc/
  # * https://pkgs.alpinelinux.org/packages?name=musl&branch=v3.13
  # * http://git.musl-libc.org/cgit/musl/commit/?h=v1.2.2
  #
  # The newest one is 10.2.1.
  MUSLCC='10.2.1'
  ARCHIVE="https://more.musl.cc/$MUSLCC/x86_64-linux-musl/${GCC_HOST_TRIPLE}-cross.tgz"

  apt-get install -y --no-install-recommends ca-certificates curl rsync
  curl -fsSL $ARCHIVE | tar -xz -C /tmp
  rm -f $(find /tmp/${GCC_HOST_TRIPLE}-cross -name "ld-musl-*.so.1")
  rm /tmp/${GCC_HOST_TRIPLE}-cross/usr
  rsync --ignore-errors -rLaq /tmp/${GCC_HOST_TRIPLE}-cross/* / || true
  rm -rf /tmp/${GCC_HOST_TRIPLE}-cross
fi
