# Docker

This folder contains files for Docker support.

## How to build images from source

Using [compose.yaml](./compose.yaml):

```shell
docker buildx bake --load
```

Using `Dockerfile.*`:

```shell
docker buildx build -t mirakc-sample -f Dockerfile.debian --target mirakc \
  --build-arg DEBIAN_CODENAME=bookworm ..
```

Use `--platform` option if you want to build a multi-arch image or cross-build
a image for a target:

```shell
docker buildx build -t mirakc-sample -f Dockerfile.debian --target mirakc \
  --build-arg DEBIAN_CODENAME=bookworm \
  --platform=linux/amd64,linux/arm/v7,linux/arm64/v8 ..
```

You cannot use `--load` option when building multi-arch images.  See
[this comment](https://github.com/docker/buildx/issues/59#issuecomment-659303756)
in docker/buildx#59.

## mirakc/buildenv

A [mirakc/buildenv] image is used as a build environment for each target platform.

[mirakc/buildenv] images on Docker Hub can be updated by running
[//scripts/update-buildenv-images](../scripts/update-buildenv-images).

[mirakc/buildenv] images were introduced in order to reduce network traffic to
[musl.cc].  See https://github.com/orgs/community/discussions/27906 for details.

## mirakc/tools

[mirakc/tools] is a multi-arch image which contains the following commands:

* mirakc-arib
* recdvb
* recpt1

[mirakc/tools] images on Docker Hub can be updated by running
[//scripts/update-tools-images](../scripts/update-tools-images).

[mirakc/tools] images were introduced in order to solve timeout issues in
`docker` jobs of GitHub Actions workflows.

[mirakc/buildenv]: https://hub.docker.com/r/mirakc/buildenv
[mirakc/tools]: https://hub.docker.com/r/mirakc/tools
[musl.cc]: https://musl.cc/
