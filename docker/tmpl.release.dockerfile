# mirakc
FROM rust:slim-buster AS mirakc-build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq
RUN apt-get install -y -qq --no-install-recommends \
    ca-certificates curl {{GXX}}

RUN rustup target add {{RUST_TARGET_TRIPLE}}

RUN mkdir -p /build
WORKDIR /build

ADD . ./
# See: https://github.com/japaric/rust-cross
#      https://doc.rust-lang.org/cargo/reference/config.html
RUN mkdir -p .cargo
RUN echo "[target.{{RUST_TARGET_TRIPLE}}]" >.cargo/config
RUN echo "linker = \"{{RUST_LINKER}}\"" >>.cargo/config
RUN cargo build --release --target {{RUST_TARGET_TRIPLE}}
RUN cp /build/target/{{RUST_TARGET_TRIPLE}}/release/mirakc /usr/local/bin/


# final image
FROM {{ARCH}}/debian:buster-slim
LABEL maintainer="Masayuki Nagamachi <masnagam@gmail.com>"

COPY --from=recdvb-build /usr/local/bin/recdvb /usr/local/bin/
COPY --from=recpt1-build /usr/local/bin/recpt1 /usr/local/bin/
COPY --from=mirakc-arib-build /build/bin/mirakc-arib /usr/local/bin/
COPY --from=mirakc-build /usr/local/bin/mirakc /usr/local/bin/

RUN set -eux \
 && export DEBIAN_FRONTEND=noninteractive \
 && apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends ca-certificates curl socat \
 # cleanup
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* \
 && rm -rf /var/tmp/* \
 && rm -rf /tmp/*

ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
EXPOSE 40772
VOLUME ["/var/lib/mirakc/epg"]
ENTRYPOINT ["mirakc"]
CMD []
