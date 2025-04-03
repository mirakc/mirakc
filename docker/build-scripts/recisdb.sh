set -eu

REPO_API_URL="https://api.github.com/repos/kazuki0824/recisdb-rs/releases/latest"
ARCH=$(dpkg --print-architecture)
RELEASE_DATA=$(curl -s $REPO_API_URL)
DEB_URL=$(echo "$RELEASE_DATA" | jq -r ".assets[] | select(.name | contains(\"$ARCH\")) | .browser_download_url")

if [ -z "$DEB_URL" ]; then
    echo "Package not found."
    exit 1
fi

curl -Lso/recisdb.deb "$DEB_URL"
apt-get install -y --no-install-recommends /recisdb.deb
rm -rf /recisdb.deb