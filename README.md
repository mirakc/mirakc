# mirakc

> A Mirakurun-compatible PVR backend written in Rust

[![ci-status](https://github.com/mirakc/mirakc/workflows/CI/badge.svg)](https://github.com/mirakc/mirakc/actions?workflow=CI)
[![coverage](https://codecov.io/gh/mirakc/mirakc/branch/main/graph/badge.svg?token=EP5rQLo3Rv)](https://codecov.io/gh/mirakc/mirakc)
[![alpine-size](https://img.shields.io/docker/image-size/mirakc/mirakc/alpine?label=alpine)](https://hub.docker.com/r/mirakc/mirakc/tags?page=1&name=alpine)
[![debian-size](https://img.shields.io/docker/image-size/mirakc/mirakc/debian?label=debian)](https://hub.docker.com/r/mirakc/mirakc/tags?page=1&name=debian)

## Quick Start

If you have already built a TV recording system with [Mirakurun] and
[EPGStation], you can simply replace Mirakurun with mirakc.

It's recommended to use a Docker image which can be downloaded from [Docker Hub].
See [docs/docker.md](./docs/docker.md) about how to make a custom Docker image
for your environment.

### Launch a mirakc Docker container

Create `config.yml`:

```yaml
epg:
  cache-dir: /var/lib/mirakc/epg

server:
  addrs:
    - !http '0.0.0.0:40772'

channels:
  # Add channels of interest.
  - name: NHK
    type: GR
    channel: '27'

tuners:
  # Add tuners available on a local machine.
  - name: Tuner0
    types: [GR]
    command: >-
      recpt1 --device /dev/px4video2 {{{channel}}} - -

filter:
  # Optionally, you can specify a command to process TS packets before sending
  # them to a client.
  #
  # The following command processes TS packets on a remote server listening on
  # TCP port 40774.
  decode-filter:
    command: >-
      socat - tcp-connect:remote:40774
```

Create `docker-compose.yml`:

```yaml
version: '3'

services:
  mirakc:
    image: docker.io/mirakc/mirakc:alpine
    container_name: mirakc
    init: true
    restart: unless-stopped
    devices:
      # Add device files used in `tuners[].command`.
      - /dev/px4video2
    ports:
      - 40772:40772
    volumes:
      - mirakc-epg:/var/lib/mirakc/epg
      - ./config.yml:/etc/mirakc/config.yml:ro
    environment:
      TZ: Asia/Tokyo
      RUST_LOG: info

volumes:
  mirakc-epg:
    name: mirakc_epg
    driver: local
```

And then launch a mirakc container:

```shell
docker compose up
```

You can also launch a mirakc container by using other tools like `podman` and
`podman-compose` which supports Docker-compatible images and
`docker-compose.yml`:

```shell
podman-compose up
```

You should see the version string when running the following command if the
mirakc starts properly:

```console
$ curl -fsSL http://localhost:40772/api/version
{"current":"<version-string>","latest":"<version-string>"}
```

See [docs/config.md](./docs/config.md) for details of `config.yml`.

## Motivation

In these days, you can build a TV recording system by yourself, using a SBC
(single-board computer) and OSS (open-source software).

Usually, SBCs are low power consumption.  For this reason, SBCs are suitable for
systems which works all the time.  On the other hand, SBCs are not so powerful
unlike PCs in these days.  Typically, a SBC have a multi-core CPU with low clock
of less than 2GHz, and only 1..4GB memory.

[Mirakurun] is one of popular DTV tuner servers in Japan.  It's fast enough, but
consumes much memory.  For example, it consumes more than 150MB even when idle.

When Mirakurun provides 8 TS streams simultaneously,  it consumes nearly 1GB of
memory which includes memory consumption of descendant processes.  As a result,
some of processes which was spawned by tuner commands will be killed in minutes
if Mirakurun is executed on a machine like ROCK64 (DRAM: 1GB) which doesn't have
enough memory.

mirakc is more efficient than Mirakurun.  It can provide at least 8 TS streams
at the same time even on ROCK64 (DRAM: 1GB).

## Performance Comparison

### After running for 1 day

The following table is a snippet of the result of `docker stats` at idle after
running for 1 day on ROCK64 (DRAM: 4GB):

```
NAME           MEM USAGE / LIMIT    MEM %  NET I/O         BLOCK I/O
mirakc-alpine  4.723MiB / 3.882GiB  0.12%  198MB / 508kB   803kB / 8.19kB
mirakc-debian  6.031MiB / 3.882GiB  0.15%  182MB / 454kB   1.43MB / 8.19kB
mirakurun      115.1MiB / 3.882GiB  2.89%  92.3GB / 219MB  0B / 996MB
```

The environment variable `MALLOC_ARENA_MAX=2` is specified in containers.

The `mirakurun` container installed a few packages at the start time.  That increased the amount
of I/O data but it seems to be less than several GB.

### 8 TS streams at the same time

mirakc is lower CPU usage and 1/40 smaller memory consumption than Mirakurun:

|          | mirakc/1.0.0 (Alpine) | mirakc/1.0.0 (Debian) | Mirakurun/3.6.0  |
|----------|-----------------------|-----------------------|------------------|
| CPU      | +46..47%              | +46..53%              | +52..59%         |
| Memory   | +12..14MB             | +20..21MB             | +865MB..3345MB   |
| Load1    | +1.71..2.55           | +1.67..2.75           | +1.52..2.90      |
| TX       | +131..137Mbps         | +118..134Mbps         | +74..119Mbps     |

where `+` means that the value is an amount of increase from its stationary value.

See [mirakc/performance-measurements] about how to collect the performance metrics.

## Limitations

* `CS` and `SKY` channel types are not tested at all
  * In addition, no pay-TV channels are tested because I have no subscription
    for pay-TV
* mirakc doesn't work with [BonDriver_Mirakurun] at this moment
  * Use [BonDriver_mirakc] instead
  * See the issue #4 for details

## TODO

* Use multiple tuners in the EPG task in order to reduce the time
  * Currently, it takes about 16 minutes for collecting EIT sections of 8 GR
    channels and 10 BS channels

## Acknowledgments

mirakc is implemented based on knowledge gained from the following software
implementations:

* [Mirakurun]

## License

Licensed under either of

* Apache License, Version 2.0
  ([LICENSE-APACHE] or http://www.apache.org/licenses/LICENSE-2.0)
* MIT License
  ([LICENSE-MIT] or http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[Mirakurun]: https://github.com/Chinachu/Mirakurun
[EPGStation]: https://github.com/l3tnun/EPGStation
[Docker Hub]: https://hub.docker.com/r/mirakc/mirakc
[mirakc/performance-measurements]: https://github.com/mirakc/performance-measurements
[BonDriver_Mirakurun]: https://github.com/Chinachu/BonDriver_Mirakurun
[BonDriver_mirakc]: https://github.com/epgdatacapbon/BonDriver_mirakc
[LICENSE-APACHE]: ./LICENSE-APACHE
[LICENSE-MIT]: ./LICENSE-MIT
