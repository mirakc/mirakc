set -eu

BASEDIR=$(cd $(dirname $0); pwd)
BUILDPLATFORM=$1
TARGETPLATFORM=$2

. $BASEDIR/vars.sh

dpkg --add-architecture $DEBIAN_ARCH
apt-get update
apt-get install -y --no-install-recommends $BUILD_DEPS

rustup target add $RUST_TARGET_TRIPLE

# cleanup
apt-get clean
rm -rf /var/lib/apt/lists/*
rm -rf /var/tmp/*
rm -rf /tmp/*
