set -eu

PROGNAME=$(basename $0)
BASEDIR=$(cd $(dirname $0); pwd)
PROJDIR=$(cd $BASEDIR/..; pwd)

REGS='docker.io'  #REGS='docker.io ghcr.io'
IMAGE=mirakc/tools
PLATFORMS='linux/386,linux/amd64,linux/arm/v5,linux/arm/v7,linux/arm64/v8'

if [ "$(uname)" != Linux ] || id -nG | grep -q docker
then
  DOCKER='docker'
else
  DOCKER='sudo docker'
fi

NO_CACHE=
PUSH_OPT=
VERSION=

help() {
  cat <<EOF >&2
Update $IMAGE images.

USAGE:
  $PROGNAME [options] <debian-codename> [<version>]
  $PROGNAME -h | --help

OPTIONS:
  -h, --help
    Show help.

  --no-cache
    Don't use the Docker buildx cache.

  --push
    Push images.

ARGUMENTS:
  <debian-codename>
    Debian codename such as trixie.

  <version>
    Optional version number.

DESCRIPTION:
  This script build $IMAGE images and optionally push them to Docker registries.

  The base image of each $IMAGE:debian will be determined automatically from the
  debian version of the mirakc/buildenv image specified in Dockerfile.tools.

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

if [ $# -eq 0 ]
then
  error '<debian-codename> is requried'
fi

DEBIAN_CODENAME="$1"

if [ $# -gt 1 ]
then
  VERSION="$2"
fi

# Use a base image other than `scratch` in order to make it possible to run
# commands with `docker run mirakc/tools:<tag> <command>`.
BASE_IMAGE="debian:$DEBIAN_CODENAME-slim"

IMG="$IMAGE:debian-$DEBIAN_CODENAME"
if [ -n "$VERSION" ]
then
  IMG="$IMG-$VERSION"
fi

for REG in $REGS
do
  log "Updating $REG/$IMG..."
  $DOCKER buildx build \
    -t $REG/$IMG \
    -f $PROJDIR/docker/Dockerfile.tools \
    --platform="$PLATFORMS" \
    --build-arg="DEBIAN_CODENAME=$DEBIAN_CODENAME" \
    --build-arg="BASE_IMAGE=$BASE_IMAGE" \
    $NO_CACHE $PUSH_OPT $PROJDIR
done
