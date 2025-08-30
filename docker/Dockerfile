# This file works properly only with Docker.
# Assumed that docker.io registry is used.
#
# `podman buildx build` doesn't work at this point because it doesn't support
# the following build arguments that docker set automagically:
#
#   BUILDPLATFORM
#   TARGETPLATFORM
#   TARGETOS
#   TARGETARCH
#   TARGETVARIANT

# We set `sid` here in order to avoid the `InvalidDefaultArgInFrom` warning,
# but this argument is always specified in build commands.  As a result, `sid`
# is never used.
ARG DEBIAN_CODENAME=sid

# Debian image stages

FROM mirakc/tools:debian-$DEBIAN_CODENAME AS mirakc-tools

FROM --platform=$BUILDPLATFORM mirakc/buildenv:debian-$DEBIAN_CODENAME-$TARGETOS-$TARGETARCH$TARGETVARIANT AS mirakc-build
COPY . ./
ARG BUILDPLATFORM
ARG TARGETPLATFORM
ARG PROFILE=release
RUN sh ./docker/build-scripts/mirakc.sh $BUILDPLATFORM $TARGETPLATFORM $PROFILE

FROM debian:$DEBIAN_CODENAME-slim AS mirakc-debian
LABEL maintainer="Contributors of mirakc"
COPY --from=mirakc-tools /usr/local/bin /usr/local/bin/
COPY --from=mirakc-build /usr/local/bin/mirakc /usr/local/bin/
COPY --from=mirakc-build /build/resources/strings.yml /etc/mirakc/strings.yml
ARG PROFILE=release
RUN --mount=type=bind,source=./docker/build-scripts,target=/build-scripts/ \
    sh /build-scripts/debian.sh mirakc $PROFILE
ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
ENV MIRAKC_BUILD_PROFILE=$PROFILE
EXPOSE 40772
ENTRYPOINT ["/usr/local/bin/mirakc"]
CMD []

FROM debian:$DEBIAN_CODENAME-slim AS timeshift-fs-debian
LABEL maintainer="Contributors of mirakc"
COPY --from=mirakc-build /usr/local/bin/mirakc-timeshift-fs /usr/local/bin/
COPY --from=mirakc-build /usr/local/bin/run-mirakc-timeshift-fs /usr/local/bin/
COPY --from=mirakc-build /build/resources/strings.yml /etc/mirakc/strings.yml
ARG PROFILE=release
RUN --mount=type=bind,source=./docker/build-scripts,target=/build-scripts/ \
    sh /build-scripts/debian.sh timeshift-fs $PROFILE
ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
ENV MIRAKC_BUILD_PROFILE=$PROFILE
ENTRYPOINT ["/usr/local/bin/run-mirakc-timeshift-fs"]

# Archive stages
#
# In this stage, /archive.tar.gz will be created.
# /archive.tar.gz contains tools and required libraries.

FROM mirakc-debian AS mirakc-archive
RUN --mount=type=bind,source=./docker/build-scripts,target=build-scripts/ \
    sh /build-scripts/archive.sh >/archive.tar.gz

FROM timeshift-fs-debian AS timeshift-fs-archive
RUN --mount=type=bind,source=./docker/build-scripts,target=build-scripts/ \
    sh /build-scripts/archive.sh >/archive.tar.gz

# Alpine image stages
#
# In this stage, binaries will be extracted from archive.tar.gz coming from the archive stage.
# This means that the alpine image reuses binaries built for the debian image.

FROM alpine AS mirakc-alpine
LABEL maintainer="Contributors of mirakc"
ARG PROFILE=release
RUN --mount=type=bind,from=mirakc-archive,source=/archive.tar.gz,target=/archive.tar.gz \
    --mount=type=bind,source=./docker/build-scripts,target=/build-scripts/ \
    sh /build-scripts/alpine.sh mirakc $PROFILE
ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
ENV MIRAKC_BUILD_PROFILE=$PROFILE
EXPOSE 40772
ENTRYPOINT ["/usr/local/bin/mirakc"]
CMD []

FROM alpine AS timeshift-fs-alpine
LABEL maintainer="Contributors of mirakc"
ARG PROFILE=release
RUN --mount=type=bind,from=timeshift-fs-archive,source=/archive.tar.gz,target=/archive.tar.gz \
    --mount=type=bind,source=./docker/build-scripts,target=/build-scripts/ \
    sh /build-scripts/alpine.sh timeshift-fs $PROFILE
ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
ENV MIRAKC_BUILD_PROFILE=$PROFILE
ENTRYPOINT ["/usr/local/bin/run-mirakc-timeshift-fs"]
