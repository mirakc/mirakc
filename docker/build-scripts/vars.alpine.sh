case "$TARGETPLATFORM" in
  'linux/386')
    GCC_HOST_TRIPLE='i686-linux-musl'
    GCC_ARCH='x86'
    RUST_TARGET_TRIPLE='i686-unknown-linux-musl'
    ;;
  'linux/amd64')
    GCC_HOST_TRIPLE='x86_64-linux-musl'
    GCC_ARCH='x86_64'
    RUST_TARGET_TRIPLE='x86_64-unknown-linux-musl'
    ;;
  'linux/arm/v6')
    GCC_HOST_TRIPLE='arm-linux-musleabi'
    GCC_ARCH='arm'
    RUST_TARGET_TRIPLE='arm-unknown-linux-musleabi'
    ;;
  'linux/arm/v7')
    GCC_HOST_TRIPLE='armv7l-linux-musleabihf'
    GCC_ARCH='arm'
    RUST_TARGET_TRIPLE='armv7-unknown-linux-musleabihf'
    ;;
  'linux/arm64/v8' | 'linux/arm64')
    GCC_HOST_TRIPLE='aarch64-linux-musl'
    GCC_ARCH='aarch64'
    RUST_TARGET_TRIPLE='aarch64-unknown-linux-musl'
    ;;
  *)
    echo "Unsupported TARGETPLATFORM: $TARGETPLATFORM" >&2
    exit 1
    ;;
esac

GCC="${GCC_HOST_TRIPLE}-gcc"
GXX="${GCC_HOST_TRIPLE}-g++"
STRIP="${GCC_HOST_TRIPLE}-strip"
