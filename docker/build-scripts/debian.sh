set -eu

IMAGE=$1
PROFILE=$2

export DEBIAN_FRONTEND=noninteractive
apt-get update

case $IMAGE in
  mirakc)
    apt-get install -y --no-install-recommends ca-certificates curl dvb-tools jq socat
    ;;
  timeshift-fs)
    apt-get install -y --no-install-recommends fuse3
    ;;
esac

if [ $PROFILE = perf ]
then
  apt-get install -y --no-install-recommends heaptrack valgrind
fi

# cleanup
apt-get clean
rm -rf /var/lib/apt/lists/*
rm -rf /var/tmp/*
rm -rf /tmp/*
