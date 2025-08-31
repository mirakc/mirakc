set -eu

BASEDIR=$(cd $(dirname $0); pwd)
BUILDPLATFORM=$1
TARGETPLATFORM=$2

. $BASEDIR/vars.sh

TARBALL_URL=https://github.com/stz2012/recpt1/tarball/master
PATCH_URL=https://gist.githubusercontent.com/masnagam/263985322d1eaa5ef2a6e27d57f297d1/raw/2a935310f4521ef245edf1df89282ce5345233f5/stz2012-recpt1-cr.patch

curl $TARBALL_URL -fsSL | tar -xz --strip-components=1
cd recpt1
./autogen.sh
./configure --prefix=/usr/local --host=$GCC_HOST_TRIPLE  # without `--enable-b25`
curl $PATCH_URL -fsSL | patch -p1  # remove CR in log messages
make -j $(nproc)
make install
$STRIP /usr/local/bin/recpt1
