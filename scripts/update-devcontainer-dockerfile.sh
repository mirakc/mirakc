set -eu

PROGNAME=$(basename $0)
BASEDIR=$(cd $(dirname $0); pwd)
PROJDIR=$(cd $BASEDIR/..; pwd)
TARGET_FILE=.devcontainer/Dockerfile

if [ "$(uname)" != Linux ] || id -nG | grep -q docker
then
  DOCKER='docker'
else
  DOCKER='sudo docker'
fi

IMAGE='mcr.microsoft.com/vscode/devcontainers/rust:1'
CLEAN=no

help() {
  cat <<EOF >&2
Update Dockerfile for VSCode Remote Container.

USAGE:
  $PROGNAME [options]
  $PROGNAME -h | --help

OPTIONS:
  -h, --help
    Show help.

  -c, --clean
    Remove $IMAGE at exit.
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

clean() {
  sleep 1
  if [ "$CLEAN" = yes ]
  then
    $DOCKER image rm -f $IMAGE >/dev/null
    log "Removed $IMAGE"
  fi
  rm -f $TEMP_FILE
}

while [ $# -gt 0 ]
do
  case "$1" in
    '-h' | '--help')
      help
      ;;
    '-c' | '--clean')
      CLEAN=yes
      shift
      ;;
    *)
      break
      ;;
  esac
done

if [ "$(pwd)" != "$PROJDIR" ]
then
  error "must run in the project root"
fi

TEMP_FILE=$(mktemp)
trap "clean" EXIT INT TERM

log "Downloading $IMAGE..."
$DOCKER image pull $IMAGE

log "Getting the commit hash of rustc contained in $IMAGE..."
COMMIT_HASH=$($DOCKER run --rm $IMAGE rustc -vV | grep 'commit-hash' | cut -d ' ' -f 2)

log "Getting the path of the default toolchain contained in $IMAGE..."
TOOLCHAIN_PATH=$($DOCKER run --rm $IMAGE rustup toolchain list -v | grep '(default)' | cut -f 2)

cat <<EOF
--------------------------------------------------------------------------------
COMMIT_HASH   : $COMMIT_HASH
TOOLCHAIN_PATH: $TOOLCHAIN_PATH
--------------------------------------------------------------------------------
EOF

log "Updating sourcemap variables in $TARGET_FILE..."
# Don't use the -i option of `sed`.
# The incompatibility between macOS and GNU will cause troubles.
#
# Use `|` instead of `/` because TOOLCHAIN_PATH contains `/`.
sed -e "s|^ENV MIRAKC_DEV_RUSTC_COMMIT_HASH=.*|ENV MIRAKC_DEV_RUSTC_COMMIT_HASH=\"$COMMIT_HASH\"|" \
    -e "s|^ENV MIRAKC_DEV_RUST_TOOLCHAIN_PATH=.*|ENV MIRAKC_DEV_RUST_TOOLCHAIN_PATH=\"$TOOLCHAIN_PATH\"|" \
    $TARGET_FILE >$TEMP_FILE
mv -f $TEMP_FILE $TARGET_FILE

if git diff --quiet -- $TARGET_FILE
then
  log "Not changed"
else
  log "Updated"
  git add $TARGET_FILE
  git commit -m "build: update $TARGET_FILE"
fi
