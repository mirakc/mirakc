# GITHUB_OUTPUT must be defined as an environment variable.
set -e

REF_TYPE=$1
REF_NAME=$2
DISTRO=$3

IMAGE="{0}"  # placeholder
VERSION=$REF_NAME

MAIN_TAG=$IMAGE:$VERSION-$DISTRO
TAGS=$MAIN_TAG

if [ $DISTRO = debian ]
then
  TAGS=$TAGS,$IMAGE:$VERSION
fi
case $REF_TYPE in
  branch)
    # Assumed that the branch created from the "main" branch.
    TOOLS_TAG=$DISTRO
    ;;
  tag)
    # Always update latest image tags when a new git tag is created.
    TAGS=$TAGS,$IMAGE:$DISTRO
    if [ $DISTRO = debian ]
    then
      TAGS=$TAGS,$IMAGE:latest
    fi
    MAJOR=$(echo $VERSION | cut -d '.' -f 1)
    MINOR=$(echo $VERSION | cut -d '.' -f 2)
    TOOLS_TAG=$DISTRO-$MAJOR.$MINOR
    ;;
  *)
    ;;
esac

echo "Version: $VERSION"
echo "Main tag: $MAIN_TAG"
echo "Tags: $TAGS"
echo "Tools tag: $TOOLS_TAG"

echo "version=$VERSION" >>$GITHUB_OUTPUT
echo "main-tag=$MAIN_TAG" >>$GITHUB_OUTPUT
echo "tags=$TAGS" >>$GITHUB_OUTPUT
echo "tools-tag=$TOOLS_TAG" >>$GITHUB_OUTPUT
