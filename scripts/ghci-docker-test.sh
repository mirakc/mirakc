set -eu

PROGNAME=$(basename $0)
BASEDIR=$(cd $(dirname $0); pwd)
PROJDIR=$(cd $BASEDIR/..; pwd)

VERSION="$1"
REGISTRIES="$2"
PLATFORMS="$3"

test_images() {
  MIRAKC_IMAGE=$1
  TIMESHIFT_FS_IMAGE=$2

  for PLATFORM in $PLATFORMS
  do
    echo "Testing $MIRAKC_IMAGE for $PLATFORM..."
    docker run --rm --platform=$PLATFORM $MIRAKC_IMAGE --version
    docker run --rm --platform=$PLATFORM --entrypoint=recdvb $MIRAKC_IMAGE --version
    docker run --rm --platform=$PLATFORM --entrypoint=recpt1 $MIRAKC_IMAGE --version
    docker run --rm --platform=$PLATFORM --entrypoint=mirakc-arib $MIRAKC_IMAGE --version
    docker run --rm --platform=$PLATFORM --entrypoint=dvbv5-zap $MIRAKC_IMAGE --version

    echo "Testing $TIMESHIFT_FS_IMAGE for $PLATFORM..."
    docker run --rm --platform=$PLATFORM --entrypoint=mirakc-timeshift-fs $TIMESHIFT_FS_IMAGE --version
    sh $PROJDIR/mirakc-timeshift-fs/tests/mount/test.sh $TIMESHIFT_FS_IMAGE $PLATFORM
  done
}

for REGISTRY in $REGISTRIES
do
  test_images $REGISTRY/mirakc/mirakc:$VERSION-debian \
              $REGISTRY/mirakc/timeshift-fs:$VERSION-debian
  test_images $REGISTRY/mirakc/mirakc:$VERSION-alpine \
              $REGISTRY/mirakc/timeshift-fs:$VERSION-alpine
done

# Check libraries actually loaded in containers created from alpine images.

ld_debug() {
  docker run --rm --platform=$3 -e LD_DEBUG=files $1 --version
  docker run --rm --platform=$3 -e LD_DEBUG=files --entrypoint=recdvb $1 --version
  docker run --rm --platform=$3 -e LD_DEBUG=files --entrypoint=recpt1 $1 --version
  docker run --rm --platform=$3 -e LD_DEBUG=files --entrypoint=mirakc-arib $1 --version
  docker run --rm --platform=$3 -e LD_DEBUG=files --entrypoint=mirakc-timeshift-fs $2 --version
}

for REGISTRY in $REGISTRIES
do
  for PLATFORM in $PLATFORMS
  do
    echo "Checking libraries for $PLATFORM..."
    EXPECTED=$(ld_debug $REGISTRY/mirakc/mirakc:$VERSION-debian \
                        $REGISTRY/mirakc/timeshift-fs:$VERSION-debian \
                        $PLATFORM 2>&1 1>/dev/null | \
               grep 'calling init:' | awk '{print $4}' | sort | uniq | tr '\n' ' ')
    echo "  EXPECTED: $EXPECTED"
    ACTUAL=$(ld_debug $REGISTRY/mirakc/mirakc:$VERSION-alpine \
                      $REGISTRY/mirakc/timeshift-fs:$VERSION-alpine \
                      $PLATFORM 2>&1 1>/dev/null | \
             grep 'calling init:' | awk '{print $4}' | sort | uniq | tr '\n' ' ')
    echo "  ACTUAL: $ACTUAL"
    test "$ACTUAL" = "$EXPECTED"
  done
done
