# docker-testbed

## What's docker-testbed

docker-testbed is an integration test environment for mirakc, built using
Docker.

The docker-testbed is constructed with two containers listed below:

* epgstation-v1
  * EPGStation v1 built using [./epgstation/Dockerfile.v1](./epgstation/Docker.v1)
  * Listen tcp/8888 on the localhost
* epgstation-v2
  * EPGStation v2 built using [./epgstation/Dockerfile.v2](./epgstation/Docker.v2)
  * Listen tcp/8889 on the localhost

These containers use sqlite3 as databases.  Configuration files are contained in
[./epgstation/config](./epgstation/config).

No mirakc container is included in the docker-testbed.  A mirakc server has to
be execuded outside the docker-testbed.

## How to use

Launch containers in the background:

```shell
docker-compose up -d
```

Show operator logs:

```shell
docker logs -f mirakc-testbed-epgstation-v1
```

Show service logs:

```shell
docker exec mirakc-testbed-epgstation-v1 tail -F service.log
```

Recorded files are stored in /app/recorded folder inside containers.

```shell
docker exec mirakc-testbed-epgstation-v1 ls -l /app/recorded
```

Stop containers, and remove them together with networks and volumes:

```shell
docker-compose down -v
```
