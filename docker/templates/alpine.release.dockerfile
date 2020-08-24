# mirakc
FROM buildenv AS mirakc-build

# Enforce to use a specific compiler in the cc crate.
ENV CC='{GCC_HOST_TRIPLE}-gcc'

# Used for a workaround to fix the following issue:
# https://github.com/rust-lang/backtrace-rs/issues/249
ENV CFLAGS='{MIRAKC_CFLAGS}'

# Use environment variables instead of creating .cargo/config:
# https://doc.rust-lang.org/cargo/reference/config.html
# https://github.com/japaric/rust-cross#cross-compiling-with-cargo
ENV CARGO_TARGET_{CARGO_TARGET_TRIPLE}_LINKER='{GCC_HOST_TRIPLE}-gcc'

# Used for a workaround to fix the following issue:
# https://github.com/rust-lang/compiler-builtins/issues/201
ENV CARGO_TARGET_{CARGO_TARGET_TRIPLE}_RUSTFLAGS="{MIRAKC_RUSTFLAGS}"

ADD . ./
RUN cargo build -v --release --target {RUST_TARGET_TRIPLE}
RUN cp /build/target/{RUST_TARGET_TRIPLE}/release/mirakc /usr/local/bin/


# final image
FROM {ARCH}/alpine:3.12
LABEL maintainer="Contributors of mirakc"

COPY --from=recdvb-build /usr/local/bin/recdvb /usr/local/bin/
COPY --from=recpt1-build /usr/local/bin/recpt1 /usr/local/bin/
COPY --from=mirakc-arib-build /build/bin/mirakc-arib /usr/local/bin/
COPY --from=mirakc-build /usr/local/bin/mirakc /usr/local/bin/
COPY ./resources/strings.yml /etc/mirakc/strings.yml
COPY ./mirakurun.openapi.json /etc/mirakurun.openapi.json

RUN set -eux \
 && apk add --no-cache ca-certificates curl libstdc++ socat tzdata

ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
EXPOSE 40772
ENTRYPOINT ["mirakc"]
CMD []
