BASEDIR=$(cd $(dirname $0); pwd)

. $BASEDIR/vars.sh

set -eu

MIRAKC_ARIB_VERSION='0.10.1'
ARCHIVE="https://github.com/mirakc/mirakc-arib/archive/$MIRAKC_ARIB_VERSION.tar.gz"

curl -fsSL $ARCHIVE | tar -xz --strip-components=1

cat <<EOF >toolchain.cmake
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR $GCC_ARCH)
set(CMAKE_C_COMPILER $GCC)
set(CMAKE_C_COMPILER_TARGET $GCC_HOST_TRIPLE)
set(CMAKE_CXX_COMPILER $GXX)
set(CMAKE_CXX_COMPILER_TARGET $GCC_HOST_TRIPLE)
EOF

cmake -B. -S. -DCMAKE_BUILD_TYPE=Release -DCMAKE_TOOLCHAIN_FILE=toolchain.cmake

if [ "$USE_MUSL" = yes ]; then
  # See https://gist.github.com/uru2/cb3f7b553c2c58570ca9bf18e47cebb3
  CPPFLAGS='-Wno-error=zero-as-null-pointer-constant' make -j $(nproc) vendor
else
  make -j $(nproc) vendor
fi
make -j $(nproc)
