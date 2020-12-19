USE_MUSL=no
MIRAKC_CFLAGS=
MIRAKC_RUSTFLAGS=

case "$TARGETPLATFORM" in
  'linux/amd64')
    GCC_HOST_TRIPLE='x86_64-linux-gnu'
    GCC_ARCH='x86_64'
    GCC='gcc'
    GXX='g++'
    RUST_TARGET_TRIPLE='x86_64-unknown-linux-gnu'
    BUILD_DEPS="$BUILD_DEPS g++"
    ;;
  'linux/arm/v5')
    GCC_HOST_TRIPLE='arm-linux-gnueabi'
    GCC_ARCH='arm'
    GCC="${GCC_HOST_TRIPLE}-gcc"
    GXX="${GCC_HOST_TRIPLE}-g++"
    RUST_TARGET_TRIPLE='arm-unknown-linux-gnueabi'
    BUILD_DEPS="$BUILD_DEPS g++-${GCC_HOST_TRIPLE}"
    ;;
  'linux/arm/v7')
    GCC_HOST_TRIPLE='arm-linux-gnueabihf'
    GCC_ARCH='arm'
    GCC="${GCC_HOST_TRIPLE}-gcc"
    GXX="${GCC_HOST_TRIPLE}-g++"
    RUST_TARGET_TRIPLE='arm-unknown-linux-gnueabihf'
    BUILD_DEPS="$BUILD_DEPS g++-${GCC_HOST_TRIPLE}"
    ;;
  'linux/arm64')
    GCC_HOST_TRIPLE='aarch64-linux-gnu'
    GCC_ARCH='aarch64'
    GCC="${GCC_HOST_TRIPLE}-gcc"
    GXX="${GCC_HOST_TRIPLE}-g++"
    RUST_TARGET_TRIPLE='aarch64-unknown-linux-gnu'
    BUILD_DEPS="$BUILD_DEPS g++-${GCC_HOST_TRIPLE}"
    ;;
  *)
    echo "Unsupported TARGETPLATFORM: $TARGETPLATFORM" >&2
    exit 1
    ;;
esac
