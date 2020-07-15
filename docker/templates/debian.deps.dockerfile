# recdvb configuired without `--enable-b25`
FROM buildenv AS recdvb-build

RUN curl -fsSL http://www13.plala.or.jp/sat/recdvb/recdvb-{RECDVB}.tgz \
    | tar -xz --strip-components=1
RUN ./autogen.sh
RUN ./configure --prefix=/usr/local --host={GCC_HOST_TRIPLE}
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
RUN ./configure --prefix=/usr/local --host={GCC_HOST_TRIPLE}
RUN curl -fsSL https://gist.githubusercontent.com/masnagam/263985322d1eaa5ef2a6e27d57f297d1/raw/2a935310f4521ef245edf1df89282ce5345233f5/stz2012-recpt1-cr.patch \
    | patch -p1  # remove CR in log messages
RUN make -j $(nproc)
RUN make install


# mirakc-arib
FROM buildenv AS mirakc-arib-build

RUN curl -fsSL https://github.com/masnagam/mirakc-arib/archive/{MIRAKC_ARIB_VERSION}.tar.gz \
    | tar -xz --strip-components=1
RUN echo 'set(CMAKE_SYSTEM_NAME Linux)' >toolchain.cmake
RUN echo 'set(CMAKE_SYSTEM_PROCESSOR {DEBIAN_ARCH})' >>toolchain.cmake
RUN echo 'set(CMAKE_C_COMPILER {GCC})' >>toolchain.cmake
RUN echo 'set(CMAKE_C_COMPILER_TARGET {GCC_HOST_TRIPLE})' >>toolchain.cmake
RUN echo 'set(CMAKE_CXX_COMPILER {GXX})' >>toolchain.cmake
RUN echo 'set(CMAKE_CXX_COMPILER_TARGET {GCC_HOST_TRIPLE})' >>toolchain.cmake
RUN cmake -B. -S. -DCMAKE_BUILD_TYPE=Release -DCMAKE_TOOLCHAIN_FILE=toolchain.cmake
RUN make -j $(nproc) vendor
RUN make -j $(nproc)
