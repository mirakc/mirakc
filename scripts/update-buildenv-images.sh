set -eu

PROGNAME=$(basename $0)
BASEDIR=$(cd $(dirname $0); pwd)
PROJDIR=$(cd $BASEDIR/..; pwd)

REGS='docker.io'  #REGS='docker.io ghcr.io'
IMAGE=mirakc/buildenv
PLATFORMS='linux/386 linux/amd64 linux/arm/v5 linux/arm/v7 linux/arm64/v8'

if [ "$(uname)" != Linux ] || id -nG | grep -q docker
then
  DOCKER='docker'
else
  DOCKER='sudo docker'
fi

NO_CACHE=
PUSH_OPT='--load'

help() {
  cat <<EOF >&2
Update $IMAGE images.

USAGE:
  $PROGNAME [--push] <debian-codename>
  $PROGNAME -h | --help

OPTIONS:
  -h, --help
    Show help.

  --no-cache
    Don't use the Docker build cache.

  --push
    Push images.

DESCRIPTION:
  This script build $IMAGE images and optionally push them to Docker registries.

  You have to login to the registries by 'docker login' before running this
  script with '--push'.
EOF
  exit 0
}

log() {
  echo "$1" >&2
}

error() {
  log "ERROR: $1"
  exit 1
}

tag_name() {
  case "$2" in
    linux/386)
      echo "debian-$1-linux-386"
      ;;
    linux/amd64)
      echo "debian-$1-linux-amd64"
      ;;
    linux/arm/v5)
      echo "debian-$1-linux-armv5"
      ;;
    linux/arm/v7)
      echo "debian-$1-linux-armv7"
      ;;
    linux/arm64/v8)
      # `docker build` doesn't define TARGETVARIANT
      echo "debian-$1-linux-arm64"
      ;;
  esac
}

while [ $# -gt 0 ]
do
  case "$1" in
    '-h' | '--help')
      help
      ;;
    '--no-cache')
      NO_CACHE='--no-cache'
      shift
      ;;
    '--push')
      PUSH_OPT='--push'
      shift
      ;;
    *)
      break
      ;;
  esac
done

DEBIAN_CODENAME=$1

for PLATFORM in $PLATFORMS
do
  IMG="$IMAGE:$(tag_name $DEBIAN_CODENAME $PLATFORM)"
  for REG in $REGS
  do
    log "Updating $REG/$IMG..."
    $DOCKER build \
      -t $REG/$IMG \
      -f $PROJDIR/docker/Dockerfile.buildenv \
      --build-arg="DEBIAN_CODENAME=$DEBIAN_CODENAME" \
      --build-arg="TARGETPLATFORM=$PLATFORM" \
      $NO_CACHE $PUSH_OPT $PROJDIR
  done
done
