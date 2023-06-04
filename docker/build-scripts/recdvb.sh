set -eu

BASEDIR=$(cd $(dirname $0); pwd)
DISTRO=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3

. $BASEDIR/vars.sh

TARBALL_URL=https://github.com/epgdatacapbon/recdvb/tarball/master

curl $TARBALL_URL -fsSL | tar -xz --strip-component=1
./autogen.sh
./configure --prefix=/usr/local --host=$GCC_HOST_TRIPLE
sed -i -e 's/msgbuf/_msgbuf/' recpt1core.h
sed -i '1i#include <sys/types.h>' tsmain.c
sed -i '1i#include <sys/types.h>' tssplitter_lite.c
if [ "$DISTRO" = alpine ]; then
  sed -i -e 's/^LDFLAGS  =$/LDFLAGS = -static -no-pie/' Makefile
fi
make -j $(nproc)
make install
$STRIP /usr/local/bin/recdvb
