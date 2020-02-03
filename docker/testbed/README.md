# docker-testbed

## What's docker-testbed

docker-testbed is an integration test environment for mirakc, built using
Docker.

The docker-testbed is constructed with several containers listed below:

* [epgstation]
  * A front server which provides UI for recording reservation using Mirakurun
* postgres
  * A Postgresql server used from EPGStation

No mirakc container is included in the docker-testbed.  A mirakc server has to
be execuded outside the docker-testbed.

## How to use

Create `.env` file in this folder, which defines the following environment
variables:

```console
$ cat .env
TZ=Asia/Tokyo
EPGSTATION_RECORDED_PATH=/path/to/epgstation/recorded
```

Create containers with networks and volumes, and start them in the background:

```console
$ docker-compose up -d
```

Show logs:

```console
$ docker-compose logs -f --tail=10
```

Stop containers, and remove them together with networks and volumes:

```console
$ docker-compose down -v
```

Recorded files are stored into the [epgstation/recorded](./epgstation/recorded)
folder.

[epgstation]: https://github.com/l3tnun/EPGStation
