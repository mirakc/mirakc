set -eu

BASEDIR=$(cd $(dirname $0); pwd)
TARGET=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3

. $BASEDIR/vars.sh

apt-get update
apt-get install -y --no-install-recommends $BUILD_DEPS

rustup target add $RUST_TARGET_TRIPLE

if [ "$USE_MUSL" = yes ]; then
  # Use a cross-compiler working with musl@1.1.24 used in alpine:3.12.
  #
  # * https://musl.cc/
  # * https://pkgs.alpinelinux.org/packages?name=musl&branch=v3.12
  # * http://git.musl-libc.org/cgit/musl/commit/?h=v1.1.24
  #
  # The newest one is 9.2.1-20191012.
  MUSLCC='9.2.1-20191012'
  ARCHIVE="https://more.musl.cc/$MUSLCC/x86_64-linux-musl/${GCC_HOST_TRIPLE}-cross.tgz"

  apt-get install -y --no-install-recommends ca-certificates curl rsync
  curl -fsSL $ARCHIVE | tar -xz -C /tmp
  rm -f $(find /tmp/${GCC_HOST_TRIPLE}-cross -name "ld-musl-*.so.1")
  rm /tmp/${GCC_HOST_TRIPLE}-cross/usr
  rsync --ignore-errors -rLaq /tmp/${GCC_HOST_TRIPLE}-cross/* / || true
  rm -rf /tmp/${GCC_HOST_TRIPLE}-cross
fi
