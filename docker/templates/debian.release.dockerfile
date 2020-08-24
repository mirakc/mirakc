# mirakc
FROM buildenv AS mirakc-build

ENV CARGO_TARGET_{CARGO_TARGET_TRIPLE}_LINKER='{GCC}'

ADD . ./
RUN cargo build -v --release --target {RUST_TARGET_TRIPLE}
RUN cp /build/target/{RUST_TARGET_TRIPLE}/release/mirakc /usr/local/bin/


# final image
#
# There is no arm32v6/debian image.  So, we use DEBIAN_ARCH instead of ARCH.
FROM {DEBIAN_ARCH}/debian:buster-slim
LABEL maintainer="Contributors of mirakc"

COPY --from=recdvb-build /usr/local/bin/recdvb /usr/local/bin/
COPY --from=recpt1-build /usr/local/bin/recpt1 /usr/local/bin/
COPY --from=mirakc-arib-build /build/bin/mirakc-arib /usr/local/bin/
COPY --from=mirakc-build /usr/local/bin/mirakc /usr/local/bin/
COPY ./resources/strings.yml /etc/mirakc/strings.yml
COPY ./mirakurun.openapi.json /etc/mirakurun.openapi.json

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
ENTRYPOINT ["mirakc"]
CMD []
