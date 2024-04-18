# mirakc

> A Mirakurun-compatible PVR backend written in Rust

[![ci](https://github.com/mirakc/mirakc/actions/workflows/ci.yml/badge.svg)](https://github.com/mirakc/mirakc/actions/workflows/ci.yml)
[![coverage](https://codecov.io/gh/mirakc/mirakc/branch/main/graph/badge.svg?token=EP5rQLo3Rv)](https://codecov.io/gh/mirakc/mirakc)
[![alpine-size](https://img.shields.io/docker/image-size/mirakc/mirakc/alpine?label=alpine)](https://hub.docker.com/r/mirakc/mirakc/tags?page=1&name=alpine)
[![debian-size](https://img.shields.io/docker/image-size/mirakc/mirakc/debian?label=debian)](https://hub.docker.com/r/mirakc/mirakc/tags?page=1&name=debian)

## Features

* Mirakurun-compatible REST API
  * Enough to work with applications such as [EPGStation]
* Standalone recording
  * No need to install additional applications anymore
* On-air time tracking
  * No need to worry about recording failures anymore
* Timeshift recording
  * No need to add recording schedules anymore

## Quick Start

If you already have a TV recording system built with [Mirakurun] and
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
    - http: '0.0.0.0:40772'

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

filters:
  # Optionally, you can specify a command to process TS packets before sending
  # them to a client.
  #
  # The following command processes TS packets on a remote server listening on
  # TCP port 40774.
  decode-filter:
    command: >-
      socat -,cool-write tcp-connect:remote:40774
```

Create `docker-compose.yml`:

```yaml
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
`podman-compose` which support Docker-compatible images and
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

The following table is a snippet from the result of `docker stats` at idle after
running for 1 day on Raspberry Pi 4B (DRAM: 4GB):

```
NAME           MEM USAGE / LIMIT    MEM %  NET I/O          BLOCK I/O
mirakc-alpine  19.5MiB / 3.704GiB   0.51%  7.45GB / 38.3MB  246kB / 15.6MB
mirakc-debian  26.47MiB / 3.704GiB  0.70%  7.49GB / 34.6MB  221kB / 15.6MB
mirakurun      123.6MiB / 3.704GiB  3.26%  61.3GB / 290MB   83.5MB / 1.36GB
```

The environment variable `MALLOC_ARENA_MAX=2` is specified in containers.

The `mirakurun` container installed a few packages at the start time.  That
increased the amount of I/O data but it seems to be less than several GB.

### 8 TS streams at the same time

mirakc is lower CPU usage and 75% smaller memory consumption than Mirakurun:

|        | mirakc/2.0.0 (Alpine) | mirakc/2.0.0 (Debian) | Mirakurun/3.9.0-rc.2 |
|--------|-----------------------|-----------------------|----------------------|
| CPU    | +26..28%              | +26..28%              | +32..36%             |
| Memory | +26..27MB             | +26..27MB             | +112MB..121MB        |
| Load1  | +0.59..1.81           | +0.64..1.57           | +0.91..1.79          |
| TX     | +155..155Mbps         | +155..155Mbps         | +154..155Mbps        |

where `+` means that the value is an amount of increase from its stationary value.

See [mirakc/performance-measurements] about how to collect the performance metrics.

## Limitations

* `CS` and `SKY` channel types are not tested at all
  * In addition, no pay-TV channels are tested because I have no subscription
    for pay-TV
* mirakc doesn't work with [BonDriver_Mirakurun]
  * Use [BonDriver_mirakc] instead
  * See the issue #4 for details

## TODO

* Use multiple tuners in the EPG task in order to reduce the execution time
  * Currently, it takes about 40 minutes for collecting EIT sections of 12 GR
    services and 12 BS services

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
