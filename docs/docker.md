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
    * Contains binaries built for Debian images and required shared libraries
  * debian (main platform)
    * Based on docker.io/debian:$DEBIAN_CODENAME-slim

Platforms listed below are supported:

* linux/386 (SSP disabled)
* linux/amd64
* linux/arm/v7
* linux/arm64/v8

Each image contains the following executables other than mirakc:

* [recdvb] configured without `--enable-b25`
* [recpt1] configured without `--enable-b25`
* [mirakc-arib]
* [curl]
* [jq]
* [socat]
* [DVBv5 Tools]

## Build a custom image

### Install additional software

You can easily build a custom image which contains additional software.

Specify one of pre-built images in the `FROM` directive in `Dockerfile`.  And
then install additional software like below:

```Dockerfile
FROM docker.io/mirakc/mirakc:debian

RUN apk add --no-cache ffmpeg
...
```

### Copy mirakc from a pre-built image

If you want to create a cleaner image, you can copy only necessary files from a
pre-built image like below:

```Dockerfile
FROM docker.io/mirakc/mirakc:debian AS mirakc-image

FROM docker.io/debian:bookworm

COPY --from=mirakc-image /usr/local/bin/mirakc /usr/local/bin/

...

ENV MIRAKC_CONFIG=/etc/mirakc/config.yml
EXPOSE 40772
VOLUME ["/var/lib/mirakc/epg"]
ENTRYPOINT ["mirakc"]
CMD []
```

Make sure that the destination image has ABI-compatible libraries required for
binaries to be copied.

If you want to copy binaries into an image such as an Alpine image which uses
libc other than `glibc`, copy the binaries together with required libraries.
See [`archive.sh`](../../docker/build-scripts/archive.sh) for details.

Use an architecture-specific image if you like to cross-build a custom image.

## Build an image from source

This repository contains the following Dockerfile file for `docker build`:

* [Dockerfile](../docker/Dockerfile)

Use the `--platform` option for specifying your target platforms like blow:

```shell
docker build -t $(id -un)/mirakc:arm32v7 -f docker/Dockerfile -t mirakc-debian \
  --platform=linux/arm/v7 .
```

The following command transfers the created image to a remote docker daemon
which can be accessed using SSH:

```shell
docker save $(id -un)/mirakc:arm32v7 | docker -H ssh://remote load
```

### Support new architectures

`Dockerfile` uses scripts in [docker/build-scripts](../docker/build-scripts) for
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
[jq]: https://stedolan.github.io/jq/
[socat]: http://www.dest-unreach.org/socat/doc/socat.html
[DVBv5 Tools]:https://linuxtv.org/wiki/index.php/DVBv5_Tools
[Docker Hub]: https://hub.docker.com/r/mirakc/mirakc
