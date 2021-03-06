# Docker

This folder contains files for Docker support.

## How to build images from source

Using [docker-compose.yml](./docker-compose.yml):

```shell
docker buildx bake --load
```

Using `Dockerfile.*`:

```shell
docker buildx build -t mirakc-sample -f Dockerfile.debian ..
```

Use `--platform` option if you want to build a multi-arch image or cross-build
a image for a target:

```shell
docker buildx build -t mirakc-sample -f Dockerfile.debian \
  --platform=linux/amd64,linux/arm/v7,linux/arm64/v8 ..
```

You cannot use `--load` option when building multi-arch images.  See
[this comment](https://github.com/docker/buildx/issues/59#issuecomment-659303756)
in docker/buildx#59.
