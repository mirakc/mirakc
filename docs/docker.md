# Using Docker

## Pre-built images in Docker Hub

There are two types of pre-built images in [Docker Hub]:

* Multi-Arch images
  * docker.io/mirakc/mirakc:latest
    * Alias of docker.io/mirakc/mirakc:$LATEST_VERSION
  * docker.io/mirakc/mirakc:$DISTRO
    * Alias of docker.io/mirakc/mirakc:$LATEST_VERSION-$DISTRO
  * docker.io/mirakc/mirakc:$VERSION
    * Alias of docker.io/mirakc/mirakc:$VERSION-debian
  * docker.io/mirakc/mirakc:$VERSION-$DISTRO

Where:

* VERSION
  * main or version tags
  * LATEST_VERSION points to the latest version tag
* DISTRO
  * alpine
    * Based on docker.io/alpine:latest
  * debian (main platform)
    * Based on docker.io/debian:buster-slim

Platforms listed below are supported:

* linux/386 (SSP disabled)
* linux/amd64
* linux/arm/v6 (only for alpine)
* linux/arm/v7
* linux/arm64/v8

Each image contains the following executables other than mirakc:

* [recdvb] configured without `--enable-b25`
* [recpt1] configured without `--enable-b25`
* [mirakc-arib]
* [curl]
* [socat]

## Build a custom image

### Install additional software

You can easily build a custom image which contains additional software.

Specify one of pre-built images in the `FROM` directive in `Dockerfile`.  And
then install additional software like below:

```Dockerfile
FROM docker.io/mirakc/mirakc:alpine

RUN apk add --no-cache ffmpeg
...
```

### Copy mirakc from a pre-built image

If you want to create a cleaner image, you can copy only necessary files from a
pre-built image like below:

```Dockerfile
FROM docker.io/mirakc/mirakc:alpine AS mirakc-image

FROM docker.io/alpine

COPY --from=mirakc-image /usr/local/bin/mirakc /usr/local/bin/

...

ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
EXPOSE 40772
VOLUME ["/var/lib/mirakc/epg"]
ENTRYPOINT ["mirakc"]
CMD []
```

Don't copy binary files from a musl-based image (like alpine) into a glibc-based
image (like debian) and vice versa.  Some of binary files in the musl-based
image are dynamic-linked against `musl`.

Use an architecture-specific image if you like to cross-build a custom image.

## Build an image from source

This repository contains the following two Dockerfile files for `docker buildx`:

* [Dockerfile.alpine](../docker/Dockerfile.alpine) for alpine-based images
* [Dockerfile.debian](../docker/Dockerfile.debian) for debian-based images

Use the `--platform` option for specifying your target platforms like blow:

```shell
docker buildx build -t $(id -un)/mirakc:arm32v7 -f docker/Dockerfile.debian \
  --platform=linux/arm/v7 .
```

The following command transfers the created image to a remote docker daemon
which can be accessed using SSH:

```shell
docker save $(id -un)/mirakc:arm32v7 | docker -H ssh://remote load
```

### Support new architectures

`Dockerfile.*` uses scripts in [docker/build-scripts](../docker/build-scripts) for
cross-compiling tools like `recpt1`.  If you want to create Docker images which
have not been supported at this point, you need to modify those scripts.

`vars.*.sh` defines variables used in other scripts.  The target platform string
will be passed using the `TARGETPLATFORM` environment variable in the build time.
See the following page for details about `BUILDPLATFORM` and `TARGETPLATFORM`:

* https://docs.docker.com/buildx/working-with-buildx/#build-multi-platform-images

[recdvb]: http://cgi1.plala.or.jp/~sat/?x=entry:entry180805-164428
[recpt1]: https://github.com/stz2012/recpt1
[mirakc-arib]: https://github.com/mirakc/mirakc-arib
[curl]: https://curl.haxx.se/docs/manpage.html
[socat]: http://www.dest-unreach.org/socat/doc/socat.html
[Docker Hub]: https://hub.docker.com/r/mirakc/mirakc
