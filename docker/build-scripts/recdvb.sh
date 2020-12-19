BASEDIR=$(cd $(dirname $0); pwd)

. $BASEDIR/vars.sh

set -eu

RECDVB='1.3.2'

curl -fsSL http://www13.plala.or.jp/sat/recdvb/recdvb-$RECDVB.tgz | tar -xz --strip-component=1
./autogen.sh
./configure --prefix=/usr/local --host=$GCC_HOST_TRIPLE
sed -i -e 's/msgbuf/_msgbuf/' recpt1core.h
sed -i '1i#include <sys/types.h>' recpt1.h
make -j $(nproc)
make install
