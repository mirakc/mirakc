set -eu

BASEDIR=$(cd $(dirname $0); pwd)
IMAGE="$1"
PLATFORM="${2:-linux/amd64}"

if id -nG | grep -q docker
then
  DOCKER='docker'
else
  DOCKER='sudo docker'
fi

$DOCKER run --rm --platform=$PLATFORM \
  --cap-add SYS_ADMIN --device /dev/fuse --security-opt apparmor:unconfined \
  --mount type=bind,src=$BASEDIR/config.yml,dst=/etc/mirakc/config.yml,readonly \
  --mount type=bind,src=$BASEDIR/timeshift.json,dst=/timeshift.json,readonly \
  --mount type=bind,src=$BASEDIR/run-tests,dst=/usr/local/bin/run-mirakc-timeshift-fs \
  $IMAGE
