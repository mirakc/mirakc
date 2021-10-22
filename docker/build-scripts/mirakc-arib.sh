set -eu

BASEDIR=$(cd $(dirname $0); pwd)
TARGET=$1
BUILDPLATFORM=$2
TARGETPLATFORM=$3

. $BASEDIR/vars.sh

MIRAKC_ARIB_VERSION="0.16.5"
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

if [ "$TARGET" = alpine ]; then
  # See https://gist.github.com/uru2/cb3f7b553c2c58570ca9bf18e47cebb3
  MIRAKC_ARIB_TSDUCK_ARIB_CXXFLAGS='-Wno-error=zero-as-null-pointer-constant'
  if [ "$TARGETPLATFORM" = linux/386 ]; then
    # Disable SSP in order solve link errors.
    # See https://bugs.gentoo.org/706210
    MIRAKC_ARIB_TSDUCK_ARIB_CXXFLAGS="$MIRAKC_ARIB_TSDUCK_ARIB_CXXFLAGS -fno-stack-protector"
  fi
  # The following setting doesn't work because tsp dynamically loads plug-ins.
  #
  #   set(CMAKE_CXX_FLAGS "-static -static-libgcc -static-libstdc++")
  #
  cat <<EOF >>toolchain.cmake
set(MIRAKC_ARIB_TSDUCK_ARIB_CXXFLAGS "$MIRAKC_ARIB_TSDUCK_ARIB_CXXFLAGS" CACHE STRING "" FORCE)
set(CMAKE_EXE_LINKER_FLAGS "-static -static-libgcc -static-libstdc++")
EOF
fi

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
