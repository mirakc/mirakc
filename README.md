# mirakc

> A Mirakurun clone written in Rust

[![linux-status](https://github.com/masnagam/mirakc/workflows/Linux/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=Linux)
[![macos-status](https://github.com/masnagam/mirakc/workflows/macOS/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=macOS)
[![arm-linux-status](https://github.com/masnagam/mirakc/workflows/ARM-Linux/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=ARM-Linux)
[![docker-status](https://github.com/masnagam/mirakc/workflows/Docker/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=Docker)
[![alpine-size](https://img.shields.io/docker/image-size/masnagam/mirakc/alpine?label=Alpine)](https://hub.docker.com/repository/docker/masnagam/mirakc/tags?page=1&name=alpine)
[![debian-size](https://img.shields.io/docker/image-size/masnagam/mirakc/debian?label=Debian)](https://hub.docker.com/repository/docker/masnagam/mirakc/tags?page=1&name=debian)

## Quick Start

If you have already built a TV recording system with [Mirakurun] and
[EPGStation], you can simply replace Mirakurun with mirakc.

It's recommended to use a Docker image which can be downloaded from [DockerHub].
See [docs/docker.md](./docs/docker.md) about how to make a custom Docker image
for your environment.

### Launch a mirakc Docker container

Create `config.yml`:

```yaml
epg:
  cache-dir: /var/lib/mirakc/epg

server:
  addrs:
    - http: 0.0.0.0:40772

channels:
  # Add channels of interest.
  - name: NHK
    type: GR
    channel: '27'

tuners:
  # Add tuners available on a local machine.
  - name: Tuner0
    types: [GR]
    command: recpt1 --device /dev/px4video2 {{channel}} - -

filter:
  # Optionally, you can specify a command to process TS packets before sending
  # them to a client.
  #
  # The following command processes TS packets on a remote server listening on
  # TCP port 40774.
  decode-filter:
    command: socat - tcp-connect:remote:40774
```

Create `docker-compose.yml`:

```yaml
version: '3.7'

services:
  mirakc:
    image: masnagam/mirakc:alpine
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
docker-compose up
```

You should see the version string when running the following command if the
mirakc starts properly:

```console
$ curl -fsSL http://localhost:40772/api/version
"0.11.0"
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
running for 1 day on ROCK64 (DRAM: 1GB):

```
NAME           MEM USAGE / LIMIT    MEM %   NET I/O          BLOCK I/O
mirakurun      121.3MiB / 985.8MiB  12.31%  1.11MB / 936MB   19.6MB / 14.4GB
mirakc-debian  107.2MiB / 985.8MiB  10.87%  1.3MB / 940MB    10.9MB / 527MB
mirakc-alpine  25.82MiB / 985.8MiB   2.62%  1.28MB / 966MB   24.6kB / 994MB
```

The environment variable `MALLOC_ARENA_MAX=2` is specified in `mirakurun` and
`mirakc-debian` containers.

### 8 TS streams at the same time

mirakc is 2/3 lower CPU usage and 1/60 smaller memory consumption than
Mirakurun:

|          | mirakc/0.4.0 (Alpine) | Mirakurun/2.11.0 |
|----------|-----------------------|------------------|
| CPU      | +32..37%              | +37..60%         |
| Memory   | +11..12MB             | +470MB..800MB    |
| Load1    | +1.4..2.8             | +1.5..2.7        |
| TX       | +111..121Mbps         | +100..140Mbps    |
| RX       | +1.0..1.1Mbps         | +0.8..1.0Mbps    |

See [docs/performance-measurement.md](./docs/performance-measurement.md) about
how to collect the performance metrics.

## Limitations

* `CS` and `SKY` channel types are not tested at all
  * In addition, no pay-TV channels are tested because I have no subscription
    for pay-TV
* mirakc doesn't work with [BonDriver_Mirakurun] at this moment
  * Use [BonDriver_mirakc] instead
  * See the issue #4 for details

## TODO

* Use multiple tuners in the EGP task in order to reduce the time
  * Currently, it takes about 16 minutes for collecting EIT sections of 8 GR
    channels and 10 BS channels
* Support to collect logo data

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
[DockerHub]: https://hub.docker.com/r/masnagam/mirakc
[BonDriver_Mirakurun]: https://github.com/Chinachu/BonDriver_Mirakurun
[BonDriver_mirakc]: https://github.com/epgdatacapbon/BonDriver_mirakc
[LICENSE-APACHE]: ./LICENSE-APACHE
[LICENSE-MIT]: ./LICENSE-MIT
