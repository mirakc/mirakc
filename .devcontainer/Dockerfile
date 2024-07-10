FROM mirakc/tools:debian AS mirakc-tools

FROM mcr.microsoft.com/vscode/devcontainers/rust:1
COPY --from=mirakc-tools /usr/local/bin /usr/local/bin/
COPY ./resources/strings.yml /etc/mirakc/strings.yml
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
ENV MIRAKC_DEV_RUSTC_COMMIT_HASH="129f3b9964af4d4a709d1383930ade12dfe7c081"
ENV MIRAKC_DEV_RUST_TOOLCHAIN_PATH="/usr/local/rustup/toolchains/1.79.0-x86_64-unknown-linux-gnu"
