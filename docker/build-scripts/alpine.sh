set -eu

tar xvf /archive.tar.gz

case $1 in
  mirakc)
    apk add --no-cache ca-certificates curl jq socat tzdata v4l-utils-dvbv5
    ;;
  timeshift-fs)
    apk add --no-cache fuse3 tzdata
    echo 'user_allow_other' >/etc/fuse.conf
    ;;
esac
