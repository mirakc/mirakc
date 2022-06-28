FROM rust:slim-buster AS buildenv
ENV DISTRO=debian
ENV BUILDPLATFORM=linux/amd64
ENV TARGETPLATFORM=linux/amd64
ENV DEBIAN_FRONTEND=noninteractive
COPY ./docker/build-scripts/vars.* /build-scripts/
COPY ./docker/build-scripts/buildenv.sh /build-scripts/
RUN sh /build-scripts/buildenv.sh $DISTRO $BUILDPLATFORM $TARGETPLATFORM
RUN mkdir -p /build
WORKDIR /build

FROM buildenv AS recdvb-build
COPY ./docker/build-scripts/recdvb.sh /build-scripts/
RUN sh /build-scripts/recdvb.sh $DISTRO $BUILDPLATFORM $TARGETPLATFORM

FROM buildenv AS recpt1-build
COPY ./docker/build-scripts/recpt1.sh /build-scripts/
RUN sh /build-scripts/recpt1.sh $DISTRO $BUILDPLATFORM $TARGETPLATFORM

FROM buildenv AS mirakc-arib-build
COPY ./docker/build-scripts/mirakc-arib.sh /build-scripts/
RUN sh /build-scripts/mirakc-arib.sh $DISTRO $BUILDPLATFORM $TARGETPLATFORM

FROM mcr.microsoft.com/vscode/devcontainers/rust:1
COPY --from=recdvb-build /usr/local/bin/recdvb /usr/local/bin/
COPY --from=recpt1-build /usr/local/bin/recpt1 /usr/local/bin/
COPY --from=mirakc-arib-build /build/build/bin/mirakc-arib /usr/local/bin/
COPY ./resources/strings.yml /etc/mirakc/strings.yml
COPY ./resources/mirakurun.openapi.json /etc/mirakurun.openapi.json
RUN set -eux \
 && export DEBIAN_FRONTEND=noninteractive \
 && apt-get update \
 && apt-get install -y --no-install-recommends binutils ca-certificates curl socat \
 # cleanup
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* \
 && rm -rf /var/tmp/* \
 && rm -rf /tmp/*

# WORKAROUND
# ----------
# For the `lldb.launch.sourceMap` property defined in .vscode/settings.json, the
# following environment variables must be defined on a remote container before
# a debugger starts.
#
#   * MIRAKC_DEV_RUSTC_COMMIT_HASH
#   * MIRAKC_DEV_RUST_TOOLCHAIN_PATH
#
# The devcontainer.json has properties to define environment variables on the
# remote container.  However, none of them work properly with the CodeLLDB
# extension.  When trying to start a debug session, the following error message
# will be shown on the debug console:
#
#   Could not set source map: the replacement path doesn't exist: "<path>"
#
# even though "<path>" exists on the remote container.
#
# Directly setting the target.source-map by using
# settings.'lldb.launch.xxxCommands' also outputs the same error message.
#
# Exporting the environment variables by using the CMD instruction doesn't work.
# The environment variables are not defined on a debuggee process.  Because the
# debuggee process is NOT a child process of the init process which executes a
# script of the CMD instruction.
#
# The only way to solve this issue is providing the values for the environment
# variables from somewhere outside the system.
ENV MIRAKC_DEV_RUSTC_COMMIT_HASH="fe5b13d681f25ee6474be29d748c65adcd91f69e"
ENV MIRAKC_DEV_RUST_TOOLCHAIN_PATH="/usr/local/rustup/toolchains/1.61.0-x86_64-unknown-linux-gnu"
