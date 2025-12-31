# Docker

This folder contains files for Docker support.

## How to build images from source

Using [`compose.yaml`](./compose.yaml):

```shell
docker bake --load
```

Using `Dockerfile`:

```shell
docker build -t mirakc-sample -f Dockerfile --target mirakc-debian \
  --build-arg DEBIAN_CODENAME=trixie ..
```

Use `--platform` option if you want to build a multi-arch image or cross-build
a image for a target:

```shell
docker build -t mirakc-sample -f Dockerfile --target mirakc-debian \
  --build-arg DEBIAN_CODENAME=trixie \
  --platform=linux/amd64,linux/arm/v7,linux/arm64/v8 ..
```

You cannot use `--load` option when building multi-arch images.  See
[this comment](https://github.com/docker/buildx/issues/59#issuecomment-659303756)
in docker/buildx#59.

## mirakc/buildenv

> NOTE: `mirakc/buildenv:alpine-*` images are no longer updated.  See #2420 for the reason.

A [`mirakc/buildenv`] image is used as a build environment for each target platform.

[`mirakc/buildenv`] images on Docker Hub can be updated by running
[`//scripts/update-buildenv-images.sh`](../scripts/update-buildenv-images.sh).

## mirakc/tools

> NOTE: `mirakc/tools:alpine` image is no longer updated.  See #2420 for the reason.

[`mirakc/tools`] is a multi-arch image which contains the following commands:

* mirakc-arib
* recdvb
* recpt1

[`mirakc/tools`] images on Docker Hub can be updated by running
[`//scripts/update-tools-images.sh`](../scripts/update-tools-images.sh).

[mirakc/tools] images were introduced in order to solve timeout issues in
`docker` jobs of GitHub Actions workflows.

[`mirakc/buildenv`]: https://hub.docker.com/r/mirakc/buildenv
[`mirakc/tools`]: https://hub.docker.com/r/mirakc/tools
