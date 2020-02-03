# Prepare a base image for each build stage in order to improve the build time.
FROM rust:slim-buster AS buildenv

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq
RUN apt-get install -y -qq --no-install-recommends {{DEPS}}

RUN rustup target add {{RUST_TARGET_TRIPLE}}

RUN mkdir -p /build
WORKDIR /build
