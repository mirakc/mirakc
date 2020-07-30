# Took from https://github.com/microsoft/vscode-dev-containers/blob/master/containers/rust/.devcontainer/Dockerfile

#-------------------------------------------------------------------------------------------------------------
# Copyright (c) Microsoft Corporation. All rights reserved.
# Licensed under the MIT License. See https://go.microsoft.com/fwlink/?linkid=2090316 for license information.
#-------------------------------------------------------------------------------------------------------------

# Change sourceMap hash values in .devcontainer/devcontainer.json when using a
# different version of Rust.
FROM rust:buster

# This Dockerfile adds a non-root user with sudo access. Use the "remoteUser"
# property in devcontainer.json to use it. On Linux, the container user's GID/UIDs
# will be updated to match your local UID/GID (when using the dockerFile property).
# See https://aka.ms/vscode-remote/containers/non-root-user for details.
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID

# Avoid warnings by switching to noninteractive
ENV DEBIAN_FRONTEND=noninteractive

# Configure apt and install packages
RUN apt-get update \
    && apt-get -y install --no-install-recommends apt-utils dialog 2>&1 \
    #
    # Verify git, needed tools installed
    && apt-get -y install git openssh-client iproute2 procps lsb-release \
    #
    # Install lldb, vadimcn.vscode-lldb VSCode extension dependencies
    && apt-get install -y lldb python3-minimal libpython3.7 \
    #
    # Install Rust components
    && rustup update 2>&1 \
    && rustup component add rls rust-analysis rust-src rustfmt clippy 2>&1 \
    #
    # Create a non-root user to use if preferred - see https://aka.ms/vscode-remote/containers/non-root-user.
    && groupadd --gid $USER_GID $USERNAME \
    && useradd -s /bin/bash --uid $USER_UID --gid $USER_GID -m $USERNAME \
    # [Optional] Add sudo support for the non-root user
    && apt-get install -y sudo \
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME\
    && chmod 0440 /etc/sudoers.d/$USERNAME \
    #
    # Install additional packages required for debugging mirakc
    && apt-get install -y binutils socat \
    #
    # Clean up
    && apt-get autoremove -y \
    && apt-get clean -y \
    && rm -rf /var/lib/apt/lists/*

# Switch back to dialog for any ad-hoc use of apt-get
ENV DEBIAN_FRONTEND=dialog

# Copy executables from build stages
COPY --from=recdvb-build /usr/local/bin/recdvb /usr/local/bin/
COPY --from=recpt1-build /usr/local/bin/recpt1 /usr/local/bin/
COPY --from=mirakc-arib-build /build/bin/mirakc-arib /usr/local/bin/

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
