BASEDIR=$(cd $(dirname $0); pwd)

. $BASEDIR/vars.sh

set -eu

apt-get update
apt-get install -y --no-install-recommends $BUILD_DEPS

rustup target add $RUST_TARGET_TRIPLE

if [ "$USE_MUSL" = yes ]; then
  # Use a cross-compiler which is based on the same version of musl as alpine:3.12.
  #
  # * https://musl.cc/
  # * https://pkgs.alpinelinux.org/packages?name=musl&branch=v3.12
  #
  MUSLCC='9.3.1-20200828'
  ARCHIVE="https://more.musl.cc/$MUSLCC/x86_64-linux-musl/${GCC_HOST_TRIPLE}-cross.tgz"

  apt-get install -y --no-install-recommends ca-certificates curl rsync
  curl -fsSL $ARCHIVE | tar -xz -C /tmp
  rm -f $(find /tmp/${GCC_HOST_TRIPLE}-cross -name "ld-musl-*.so.1")
  rm /tmp/${GCC_HOST_TRIPLE}-cross/usr
  rsync --ignore-errors -rLaq /tmp/${GCC_HOST_TRIPLE}-cross/* / || true
  rm -rf /tmp/${GCC_HOST_TRIPLE}-cross
fi
