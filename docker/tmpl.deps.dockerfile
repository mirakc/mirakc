# recdvb configuired without `--enable-b25`
FROM debian:buster-slim AS recdvb-build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq
RUN apt-get install -y -qq --no-install-recommends \
    ca-certificates curl \
    autoconf automake make pkg-config {{GXX}}

RUN mkdir -p /build
WORKDIR /build
RUN curl -fsSL http://www13.plala.or.jp/sat/recdvb/recdvb-{{RECDVB}}.tgz \
    | tar -xz --strip-components=1
RUN ./autogen.sh
RUN ./configure --prefix=/usr/local {{CONFIGURE_HOST}}
RUN sed -i -e s/msgbuf/_msgbuf/ recpt1core.h
RUN sed -i '1i#include <sys/types.h>' recpt1.h
RUN make -j $(nproc)
RUN make install


# recpt1 configured without `--enable-b25`
FROM debian:buster-slim AS recpt1-build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq
RUN apt-get install -y -qq --no-install-recommends \
    ca-certificates curl \
    autoconf automake make pkg-config {{GXX}}

RUN mkdir -p /build
WORKDIR /build
RUN curl -fsSL https://github.com/stz2012/recpt1/tarball/master \
    | tar -xz --strip-components=1
WORKDIR /build/recpt1
RUN ./autogen.sh
RUN ./configure --prefix=/usr/local {{CONFIGURE_HOST}}
RUN make -j $(nproc)
RUN make install


# mirakc-arib
FROM debian:buster-slim AS mirakc-arib-build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq
RUN apt-get install -y -qq --no-install-recommends \
    ca-certificates curl \
    cmake git dos2unix make ninja-build autoconf automake libtool pkg-config \
    {{GXX}}

RUN mkdir -p /build
WORKDIR /build
RUN curl -fsSL https://github.com/masnagam/mirakc-arib/tarball/master \
    | tar -xz --strip-components=1
RUN cmake -S . -B . -G Ninja -D CMAKE_BUILD_TYPE=Release {{CMAKE_TOOLCHAIN_FILE}}
RUN ninja vendor
RUN ninja
