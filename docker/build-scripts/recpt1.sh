set -eu

BASEDIR=$(cd $(dirname $0); pwd)
TARGET=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3

. $BASEDIR/vars.sh

PATCH=https://gist.githubusercontent.com/masnagam/263985322d1eaa5ef2a6e27d57f297d1/raw/2a935310f4521ef245edf1df89282ce5345233f5/stz2012-recpt1-cr.patch

curl -fsSL https://github.com/stz2012/recpt1/tarball/master | tar -xz --strip-components=1
cd /build/recpt1
./autogen.sh
./configure --prefix=/usr/local --host=$GCC_HOST_TRIPLE  # without `--enable-b25`
curl -fsSL $PATCH | patch -p1  # remove CR in log messages
if [ "$TARGET" = alpine ]; then
  sed -i -e 's/^LDFLAGS  =$/LDFLAGS = -static -no-pie/' Makefile
fi
make -j $(nproc)
make install
$STRIP /usr/local/bin/recpt1
