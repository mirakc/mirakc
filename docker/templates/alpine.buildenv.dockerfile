# Prepare a base image for each build stage in order to improve the build time.
#
# Executables will be linked against musl dynamically.
FROM rust:slim-buster AS buildenv

ENV DEBIAN_FRONTEND=noninteractive

# Use an old cross-compiler which is based on the same version of musl as
# alpine:3.12.
#
# The latest one is based on #241263 (2019-12-20) for time64 support.  On the
# other hand, alpine:3.11 supports musl/1.1.24 which doesn't support time64.
#
# * https://musl.cc/
# * https://pkgs.alpinelinux.org/packages?name=musl&branch=v3.12
#
ARG MUSLCC_BASE_URL=https://more.musl.cc/9.2.1-20191012/x86_64-linux-musl

RUN apt-get update -qq
RUN apt-get install -y -qq --no-install-recommends {BUILD_DEPS}

RUN apt-get install -y -qq --no-install-recommends ca-certificates curl rsync
RUN curl -fsSL $MUSLCC_BASE_URL/{GCC_HOST_TRIPLE}-cross.tgz \
    | tar -xz -C /tmp
RUN rm -f $(find /tmp/{GCC_HOST_TRIPLE}-cross -name "ld-musl-*.so.1")
RUN rm /tmp/{GCC_HOST_TRIPLE}-cross/usr
RUN rsync --ignore-errors -rLaq /tmp/{GCC_HOST_TRIPLE}-cross/* / || true
RUN rm -rf /tmp/{GCC_HOST_TRIPLE}-cross

RUN rustup target add {RUST_TARGET_TRIPLE}

RUN mkdir -p /build
WORKDIR /build
