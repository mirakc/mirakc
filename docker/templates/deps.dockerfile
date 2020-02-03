# recdvb configuired without `--enable-b25`
FROM buildenv AS recdvb-build

RUN curl -fsSL http://www13.plala.or.jp/sat/recdvb/recdvb-{{RECDVB}}.tgz \
    | tar -xz --strip-components=1
RUN ./autogen.sh
RUN ./configure --prefix=/usr/local {{CONFIGURE_HOST}}
RUN sed -i -e s/msgbuf/_msgbuf/ recpt1core.h
RUN sed -i '1i#include <sys/types.h>' recpt1.h
RUN make -j $(nproc)
RUN make install


# recpt1 configured without `--enable-b25`
FROM buildenv AS recpt1-build

RUN curl -fsSL https://github.com/stz2012/recpt1/tarball/master \
    | tar -xz --strip-components=1
WORKDIR /build/recpt1
RUN ./autogen.sh
RUN ./configure --prefix=/usr/local {{CONFIGURE_HOST}}
RUN make -j $(nproc)
RUN make install


# mirakc-arib
FROM buildenv AS mirakc-arib-build

RUN curl -fsSL https://github.com/masnagam/mirakc-arib/tarball/master \
    | tar -xz --strip-components=1
RUN cmake -S . -B . -D CMAKE_BUILD_TYPE=Release {{CMAKE_TOOLCHAIN_FILE}}
RUN make -j $(nproc) vendor
RUN make -j $(nproc)
