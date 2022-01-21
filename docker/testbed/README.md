# testbed

## What's testbed

testbed is an integration test environment for mirakc, built using containers.

The testbed is constructed with two containers listed below:

* epgstation-v1
  * EPGStation v1 built using [./epgstation/Dockerfile](./epgstation/Dockerfile)
    with `TAG=v1.7.6-alpine`
  * Listen tcp/8888 on the localhost
  * Configuration files are stored in [./epgstation/config-v1](./epgstation/config-v1)
* epgstation-v2
  * EPGStation v2 built using [./epgstation/Dockerfile](./epgstation/Dockerfile)
    with `TAG=alpine`
  * Listen tcp/8889 on the localhost
  * Configuration files are stored in [./epgstation/config-v2](./epgstation/config-v2)

Both container use sqlite3.

No mirakc container is included in the testbed.  A mirakc server has to be
executed outside the testbed.  Start a mirakc server listening on
`/tmp/mirakc.sock` before launching the containers.

## How to use

Launch containers in the background:

```shell
docker compose up -d
```

Show operator logs:

```shell
docker compose logs -f epgstation-v1
```

Show service logs:

```shell
docker compose exec epgstation-v1 tail -F service.log
```

Recorded files are stored in /app/recorded folder inside containers.

```shell
docker compose exec epgstation-v1 ls -l /app/recorded
```

Stop containers, and remove them together with networks and volumes:

```shell
docker compose down -v
```

## Podman

The testbed also works property with [Podman] and [Podman Compose]:

```shell
podman-compose up -d
```

[Podman]: https://podman.io/
[Podman Compose]: https://github.com/containers/podman-compose
