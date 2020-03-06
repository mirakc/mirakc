# Using Docker

## Pre-built images in DockerHub

There are two types of pre-built images in [DockerHub]:

* Multi-Arch images
  * masnagam/mirakc:latest
    * Alias of masnagam/mirakc:$LATEST_VERSION
  * masnagam/mirakc:$PLATFORM
    * Alias of masnagam/mirakc:$LATEST_VERSION-$PLATFORM
  * masnagam/mirakc:$VERSION
    * Alias of masnagam/mirakc:$VERSION-debian
  * masnagam/mirakc:$VERSION-$PLATFORM
* Image for each architecture
  * masnagam/mirakc:$ARCH
    * Alias of masnagam/mirakc:$LATEST_VERSION-$ARCH
  * masnagam/mirack:$PLATFORM-$ARCH
    * Alias of masnagam/mirakc:$LATEST_VERSION-debian-$ARCH
  * masnagam/mirack:$VERSION-$ARCH
    * Alias of masnagam/mirakc:$VERSION-debian-$ARCH
  * masnagam/mirack:$VERSION-$PLATFORM-$ARCH

Where:

* VERSION
  * master or version tags
  * LATEST_VERSION points to the latest version tag
* PLATFORM
  * alpine
    * Based on alpine:3.11
  * debian (main platform)
    * Based on debian:buster-slim
* ARCH
  * amd64
  * arm32v6 (linux/arm/v6 for alpine, linux/arm/v5 for debian)
  * arm32v7
  * arm64v8

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
FROM masnagam/mirakc:alpine

RUN apk add --no-cache ffmpeg
...
```

Use an architecture-specific image if you like to cross-build a custom image:

```Dockerfile
FROM masnagam/mirakc:alpine-arm64v8

RUN apk add --no-cache ffmpeg
...
```

### Copy mirakc from a pre-built image

If you want to create a cleaner image, you can copy only necessary files from a
pre-built image like below:

```Dockerfile
FROM masnagam/mirakc:alpine AS mirakc-image

FROM alpine:3.11

COPY --from=mirakc-image /usr/local/bin/mirakc /usr/local/bin/
COPY --from=mirakc-image /etc/mirakurun.openapi.json /etc/

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

First, create a Dockerfile for your target platform/architecture by using
[dockerfile-gen](../docker/dockerfile-gen):

```shell
./docker/dockerfile-gen alpine arm64v8 >Dockerfile
docker build -t $(id -un)/mirakc:alpine-arm64v8 .
```

See `dockerfile-gen -h` for supported architecture.

A generated Dockerfile contains multi-stage builds for compiling the
executables.  The multi-stage builds creates untagged intermediate images like
below:

```console
$ docker images --format "{{.Repository}}:{{.Tag}}"
masnagam/mirakc:alpine-arm64v8
<none>:<none>
...
```

The following command removes **all untagged images** including the intermediate
images:

```console
$ docker images -f dangling=true -q | xargs docker rmi
```

The following command transfers the created image to a remote docker daemon
which can be accessed using SSH:

```console
$ docker save $(id -un)/alpine-mirakc:arm64v8 | docker -H ssh://remote load
```

### Support new architecture

Set properties in [docker/templates/params.json](../docker/templates/params/json)
for a new architecture you like to add.  And then create a Dockerfile for the
new architecture and build an image using it.

[recdvb]: http://cgi1.plala.or.jp/~sat/?x=entry:entry180805-164428
[recpt1]: https://github.com/stz2012/recpt1
[mirakc-arib]: https://github.com/masnagam/mirakc-arib
[curl]: https://curl.haxx.se/docs/manpage.html
[socat]: http://www.dest-unreach.org/socat/doc/socat.html
[DockerHub]: https://hub.docker.com/r/masnagam/mirakc
