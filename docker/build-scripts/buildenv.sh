set -eu

BASEDIR=$(cd $(dirname $0); pwd)
DISTRO=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3

. $BASEDIR/vars.sh

dpkg --add-architecture $DEBIAN_ARCH
apt-get update
apt-get install -y --no-install-recommends $BUILD_DEPS

rustup target add $RUST_TARGET_TRIPLE

if [ "$DISTRO" = alpine ]; then
  ARCHIVE="https://more.musl.cc/x86_64-linux-musl/${GCC_HOST_TRIPLE}-cross.tgz"

  apt-get install -y --no-install-recommends ca-certificates curl rsync
  curl -fsSL $ARCHIVE | tar -xz -C /tmp
  rm -f $(find /tmp/${GCC_HOST_TRIPLE}-cross -name "ld-musl-*.so.1")
  rm /tmp/${GCC_HOST_TRIPLE}-cross/usr
  rsync --ignore-errors -rLaq /tmp/${GCC_HOST_TRIPLE}-cross/* / || true
  rm -rf /tmp/${GCC_HOST_TRIPLE}-cross
fi

# cleanup
apt-get clean
rm -rf /var/lib/apt/lists/*
rm -rf /var/tmp/*
rm -rf /tmp/*
