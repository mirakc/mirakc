# Check preconditions
if [ "$BUILDPLATFORM" != 'linux/amd64' ]; then
  echo "Unsupported BUILDPLATFORM: $BUILDPLATFORM" >&2
  exit 1
fi

RECDVB_DEPS=$(cat <<EOF
autoconf
automake
ca-certificates
curl
make
pkg-config
EOF
)

RECPT1_DEPS=$(cat <<EOF
autoconf
automake
ca-certificates
curl
make
pkg-config
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
pkg-config
EOF
)

MIRAKC_DEPS=$(cat <<EOF
ca-certificates
curl
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
USE_MUSL
  $USE_MUSL
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
MIRACK_CFLAGS
  $MIRAKC_CFLAGS
MIRAKC_RUSTFLAGS
  $MIRAKC_RUSTFLAGS
EOF
