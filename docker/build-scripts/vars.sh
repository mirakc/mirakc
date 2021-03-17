# Check preconditions
if [ "$BUILDPLATFORM" != 'linux/amd64' ]; then
  echo "Unsupported BUILDPLATFORM: $BUILDPLATFORM" >&2
  exit 1
fi

case "$TARGETPLATFORM" in
  'linux/386')
    DEBIAN_ARCH='i386'
    ;;
  'linux/amd64')
    DEBIAN_ARCH='amd64'
    ;;
  'linux/arm/v5')
    DEBIAN_ARCH='armel'
    ;;
  'linux/arm/v7')
    DEBIAN_ARCH='armhf'
    ;;
  'linux/arm64/v8' | 'linux/arm64')
    DEBIAN_ARCH='arm64'
    ;;
  *)
    echo "Unsupported TARGETPLATFORM: $TARGETPLATFORM" >&2
    exit 1
    ;;
esac

RECDVB_DEPS=$(cat <<EOF
autoconf
automake
ca-certificates
curl
make
pkg-config:$DEBIAN_ARCH
EOF
)

RECPT1_DEPS=$(cat <<EOF
autoconf
automake
ca-certificates
curl
make
pkg-config:$DEBIAN_ARCH
EOF
)

MIRAKC_ARIB_DEPS=$(cat <<EOF
autoconf
automake
ca-certificates
curl
cmake
dos2unix
git
libtool
make
patch
pkg-config:$DEBIAN_ARCH
EOF
)

MIRAKC_DEPS=$(cat <<EOF
ca-certificates
curl
pkg-config:$DEBIAN_ARCH
libfuse-dev:$DEBIAN_ARCH
EOF
)

BUILD_DEPS=$(cat <<EOF | sort | uniq | tr '\n' ' '
$RECDVB_DEPS
$RECPT1_DEPS
$MIRAKC_ARIB_DEPS
$MIRAKC_DEPS
EOF
)

# Import target-specific variables
. $BASEDIR/vars.$TARGET.sh

cat <<EOF
BUILD_DEPS
  $BUILD_DEPS
GCC_HOST_TRIPLE
  $GCC_HOST_TRIPLE
GCC_ARCH
  $GCC_ARCH
GCC
  $GCC
GXX
  $GXX
RUST_TARGET_TRIPLE
  $RUST_TARGET_TRIPLE
EOF
