set -eu

BASEDIR=$(cd $(dirname $0); pwd)
BUILDPLATFORM=$1
TARGETPLATFORM=$2

. $BASEDIR/vars.sh

MIRAKC_ARIB_VERSION="0.24.22"
MIRAKC_ARIB_GIT_URL='https://github.com/mirakc/mirakc-arib.git'

git clone --recursive --depth=1 --branch=$MIRAKC_ARIB_VERSION $MIRAKC_ARIB_GIT_URL .

cat <<EOF >toolchain.cmake
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR $GCC_ARCH)
set(CMAKE_C_COMPILER $GCC)
set(CMAKE_C_COMPILER_TARGET $GCC_HOST_TRIPLE)
set(CMAKE_CXX_COMPILER $GXX)
set(CMAKE_CXX_COMPILER_TARGET $GCC_HOST_TRIPLE)
EOF

# Generating Makefiles for in-source build fails due to the following error:
#
#   CMake Error at vendor/docopt/docopt-config.cmake:1 (include):
#     include could not find requested file:
#
#       /home/masnagam/workspace/mirakc/mirakc-arib/vendor/docopt/docopt-targets.cmake
#   Call Stack (most recent call first):
#     CMakeLists.txt:294 (find_package)
#
# I don't know the exact reason, but generating Makefiles for out-of-source build works fine.
cmake -S . -B build -D CMAKE_BUILD_TYPE=Release -D CMAKE_TOOLCHAIN_FILE=toolchain.cmake
make -C build -j $(nproc) vendor
make -C build -j $(nproc)
$STRIP build/bin/mirakc-arib
