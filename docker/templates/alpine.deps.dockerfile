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
RUN make -j $(nproc)
RUN make install


# mirakc-arib
FROM buildenv AS mirakc-arib-build

RUN curl -fsSL https://github.com/masnagam/mirakc-arib/archive/{MIRAKC_ARIB_VERSION}.tar.gz \
    | tar -xz --strip-components=1
RUN echo 'set(CMAKE_SYSTEM_NAME Linux)' >toolchain.cmake
RUN echo 'set(CMAKE_SYSTEM_PROCESSOR arm)' >>toolchain.cmake
RUN echo 'set(CMAKE_C_COMPILER {GCC_HOST_TRIPLE}-gcc)' >>toolchain.cmake
RUN echo 'set(CMAKE_C_COMPILER_TARGET {GCC_HOST_TRIPLE})' >>toolchain.cmake
RUN echo 'set(CMAKE_CXX_COMPILER {GCC_HOST_TRIPLE}-g++)' >>toolchain.cmake
RUN echo 'set(CMAKE_CXX_COMPILER_TARGET {GCC_HOST_TRIPLE})' >>toolchain.cmake
RUN cmake -D CMAKE_BUILD_TYPE=Release -D CMAKE_TOOLCHAIN_FILE=toolchain.cmake
# See https://gist.github.com/uru2/cb3f7b553c2c58570ca9bf18e47cebb3
RUN CPPFLAGS='-Wno-error=zero-as-null-pointer-constant' make -j $(nproc) vendor
RUN make -j $(nproc)
