# Based on https://github.com/microsoft/vscode-dev-containers/blob/master/containers/alpine-3.10-git/.devcontainer/Dockerfilew

# CAUTION
# =======
#
# codelldb in vscode-lldb DOES NOT work with musl at this moment.  Because it's
# built for x86_64-unknown-linux-gnu.

#-------------------------------------------------------------------------------------------------------------
# Copyright (c) Microsoft Corporation. All rights reserved.
# Licensed under the MIT License. See https://go.microsoft.com/fwlink/?linkid=2090316 for license information.
#-------------------------------------------------------------------------------------------------------------
FROM alpine:edge

# This Dockerfile adds a non-root user with sudo access. Use the "remoteUser"
# property in devcontainer.json to use it. On Linux, the container user's GID/UIDs
# will be updated to match your local UID/GID (when using the dockerFile property).
# See https://aka.ms/vscode-remote/containers/non-root-user for details.
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID

# Set to false to skip installing zsh and Oh My ZSH!
#
# DO NOT TURN OFF!!
# The Docker image doesn't work properly when disabling INSTALL_ZSH...
ARG INSTALL_ZSH="true"

# Location and expected SHA for common setup script - SHA generated on release
ARG COMMON_SCRIPT_SOURCE="https://raw.githubusercontent.com/microsoft/vscode-dev-containers/master/script-library/common-alpine.sh"
ARG COMMON_SCRIPT_SHA="dev-mode"

# Install git, bash, dependencies, and add a non-root user
#
# Add testing repository
RUN echo "http://dl-cdn.alpinelinux.org/alpine/edge/testing" >> /etc/apk/repositories \
    && apk update \
    && apk add --no-cache wget coreutils ca-certificates \
    #
    # Verify git, common tools / libs installed, add/modify non-root user, optionally install zsh
    && wget -q -O /tmp/common-setup.sh $COMMON_SCRIPT_SOURCE \
    && if [ "$COMMON_SCRIPT_SHA" != "dev-mode" ]; then echo "$COMMON_SCRIPT_SHA /tmp/common-setup.sh" | sha256sum -c - ; fi \
    && /bin/ash /tmp/common-setup.sh "$INSTALL_ZSH" "$USERNAME" "$USER_UID" "$USER_GID" \
    && rm /tmp/common-setup.sh \
    #
    # Install lldb, vadimcn.vscode-lldb VSCode extension dependencies
    && apk add --no-cache lldb python3 \
    #
    # Install rust and components
    && apk add --no-cache curl gcc gcompat libc6-compat musl-dev \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
    && source $HOME/.cargo/env \
    && rustup update 2>&1 \
    && rustup component add rls rust-analysis rust-src rustfmt clippy 2>&1 \
    #
    # Install additional packages required for debugging mirakc
    && apk add --no-cache libstdc++ binutils socat tzdata

# Copy executables from build stages
COPY --from=recdvb-build /usr/local/bin/recdvb /usr/local/bin/
COPY --from=recpt1-build /usr/local/bin/recpt1 /usr/local/bin/
COPY --from=mirakc-arib-build /build/bin/mirakc-arib /usr/local/bin/

ENV PATH="/root/.cargo/bin:$PATH"

# See https://github.com/rust-lang/rust/pull/58575#issuecomment-496026747
ENV RUSTFLAGS="-C target-feature=-crt-static"

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
ENV MIRAKC_DEV_RUSTC_COMMIT_HASH="{RUSTC_COMMIT_HASH}"
ENV MIRAKC_DEV_RUST_TOOLCHAIN_PATH="{RUST_TOOLCHAIN_PATH}"
