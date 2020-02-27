# Use Debian-base image instead of Alpine-base image.
#
# GCC/Alpine causes build errors on aarch64 platforms.
# See https://github.com/lovell/farmhash/issues/24 for more information.
FROM node:12-buster-slim

LABEL maintainer="Masayuki Nagamachi <masnagam@gmail.com>"

RUN set -eux \
 && export DEBIAN_FRONTEND=noninteractive \
 && apt-get update -qq \
 && BUILD_DEPS="build-essential ca-certificates curl python" \
 && apt-get install -y -qq --no-install-recommends $BUILD_DEPS ffmpeg \
 # epgstation
 && mkdir -p /opt/epgstation \
 && curl -fsSL https://github.com/l3tnun/EPGStation/tarball/master | \
      tar -zx -C /opt/epgstation --strip-components=1 \
 && (cd /opt/epgstation; npm install && npm run build) \
 # cleanup
 && npm cache clean --force \
 && apt-get purge -y -qq $BUILD_DEPS \
 && apt-get autoremove -y -qq --purge \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* \
 && rm -rf /var/tmp/* \
 && rm -rf /tmp/*

EXPOSE 8888
WORKDIR /opt/epgstation
ENTRYPOINT ["node", "dist/server/index.js"]
CMD []
