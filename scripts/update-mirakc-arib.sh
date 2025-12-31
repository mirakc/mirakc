set -eu

PROJDIR=$(cd $(dirname $0)/..; pwd)
TARGET_FILE=docker/build-scripts/mirakc-arib.sh

log() {
  echo "$1" >&2
}

error() {
  log "ERROR: $1"
  exit 1
}

if [ "$(pwd)" != "$PROJDIR" ]
then
  error "must run in the project root"
fi

CURRENT=$(grep 'MIRAKC_ARIB_VERSION=' $TARGET_FILE | cut -d '=' -f 2 | tr -d '"')

VERSION="$(gh api repos/mirakc/mirakc-arib/tags --jq '.[0].name')"

TEMP_FILE=$(mktemp)
trap "rm -f $TEMP_FILE" EXIT INT TERM

# Don't use the -i option of `sed`.
# The incompatibility between macOS and GNU will cause troubles.
sed -r -e "s|^MIRAKC_ARIB_VERSION=.*|MIRAKC_ARIB_VERSION=\"$VERSION\"|" $TARGET_FILE >$TEMP_FILE
mv -f $TEMP_FILE $TARGET_FILE

if git diff --quiet -- $TARGET_FILE
then
  log "Not changed"
else
  log "Updated from $CURRENT to $VERSION"
  git add $TARGET_FILE
  git commit -m "build(deps): bump mirakc-arib from $CURRENT to $VERSION"
fi
