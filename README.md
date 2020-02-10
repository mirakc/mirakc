# mirakc

> A Mirakurun clone written in Rust

[![linux-status](https://github.com/masnagam/mirakc/workflows/Linux/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=Linux)
[![macos-status](https://github.com/masnagam/mirakc/workflows/macOS/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=macOS)
[![arm-linux-status](https://github.com/masnagam/mirakc/workflows/ARM-Linux/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=ARM-Linux)
[![docker-status](https://github.com/masnagam/mirakc/workflows/Docker/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=Docker)
[![docker-pulls](https://img.shields.io/docker/pulls/masnagam/mirakc)](https://hub.docker.com/r/masnagam/mirakc)

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

## Performance comparison

### After running for 1 day

The following table is a snippet of the result of `docker stats` at idle after
running for 1 day on ROCK64 (DRAM: 1GB):

```
NAME       MEM USAGE / LIMIT    MEM %   NET I/O          BLOCK I/O
mirakc     107.2MiB / 985.8MiB  10.87%  1.3MB / 940MB    10.9MB / 527MB
mirakurun  121.3MiB / 985.8MiB  12.31%  1.11MB / 936MB   19.6MB / 14.4GB
```

The environment variable `MALLOC_ARENA_MAX=2` is specified in both containers.

### 8 TS streams at the same time

mirakc is 2/3 lower CPU usage and 1/60 smaller memory consumption than
Mirakurun:

|          | mirakc/0.2.0  | Mirakurun/2.11.0 |
|----------|---------------|------------------|
| CPU      | +33..36%      | +37..60%         |
| Memory   | +11..12MB     | +470MB..800MB    |
| Load1    | +2.1..4.5     | +1.5..2.7        |
| TX       | +102..120Mbps | +100..140Mbps    |
| RX       | +0.9..1.1Mbps | +0.8..1.0Mbps    |

Several hundreds or thousands of dropped packets were sometimes detected during
the performance measurement.  The same situation occurred in Mirakurun.

The performance metrics listed above were collected by using the following
command executed on a local PC:

```console
$ sh ./scripts/measure.sh http://mirakc:40772 >/dev/null
Reading TS packets from mx...
Reading TS packets from cx...
Reading TS packets from ex...
Reading TS packets from tx...
Reading TS packets from bs-ntv...
Reading TS packets from bs-ex...
Reading TS packets from bs-tbs...
Reading TS packets from bs11...
CHANNEL  #BYTES      #PACKETS  #DROPS
-------  ----------  --------  ------
mx       734332700   3906025   0
cx       1029828844  5477813   0
ex       1076058232  5723714   0
tx       914647824   4865148   0
bs-ntv   1011125100  5378325   0
bs-ex    1096791248  5833996   0
bs-tbs   1030786892  5482909   0
bs11     1426981852  7590329   0

NAME    MIN                 MAX
------  ------------------  ------------------
cpu     38.36255750386465   41.38333333306946
memory  450314240           452210688
load1   2.13                4.48
tx      102581677.17847857  120792845.27698153
rx      955730.9153943596   1086586.6666666667

http://localhost:9090/graph?<query parameters for showing measurement results>
```

with the following environment:

* Tuner Server
  * ROCK64 (DRAM: 4GB)
    * The script above cannot work with Mirakurun running on ROCK64 (DRAM: 1GB)
      due to a lack of memory as described in the previous section
  * [Armbian]/Buster, Linux rock 4.4.182-rockchip64
  * [px4_drv] a1b81c3f76bab5182370cb41216bff964a24fd21@master
    * `coherent_pool=4M` is required for working with PLEX PX-Q3U4
  * Default `server.workers` (4 = the number of CPU cores)
  * `MALLOC_ARENA_MAX=2`
* Tuner
  * PLEX PX-Q3U4
* Docker
  * version 19.03.1, build 74b1e89
  * Server processes were executed in a docker container

where a Prometheus server was running on the local PC.

[scripts/measure.sh](./scripts/measure.sh) performs:

* Receiving TS streams from 4 GR and 4 BS services for 10 minutes
  * `cat` is used as post-filter
* Collecting system metrics by using [Prometheus] and [node_exporter]
* Counting the number of dropped TS packets by using [node-aribts]

The script should be run when the target server is idle.

You can spawn a Prometheus server using a `docker-compose.yml` in the
[docker/prometheus](./docker/prometheus) folder if you have never used it
before.  See [README.md](./docker/prometheus/README.md) in the folder before
using it.

## Build a TV recording system with mirakc

First of all, it's recommended you to use a Docker image which can be downloaded
from [DockerHub].  Because you need to install additional programs besides
mirakc in order to build a TV recording system.

### Using Docker

See [docker/docker-compose.yml](./docker/docker-compose.yml) and
[docker/sample-mirakc-config.yml](./docker/sample-mirakc-config.yml).

A [masnagam/docker-mirakc] Docker image for each architecture contains the
following executables other than mirakc:

* [recdvb] configured without `--enable-b25`
* [recpt1] configured without `--enable-b25`
* [mirakc-arib]
* [curl]
* [socat]

### From source

You can easily build mirakc with the following command:

```shell
cargo build --release
```

However, it takes a long time to build mirakc on a SBC.  So, it's recommended
you to cross-compile mirakc on a powerful PC.

Additionally, it's necessary to install the following programs:

* A tuner program like `recpt1`
* [mirakc-arib] which processes TS packets fed from the tuner program

These programs are specified in a configuration YAML file which is explained in
the next section.

mirakc loads a configuration in the following order:

1. From a file specified with the `-c` (or `--config`) command-line option if
   specified
2. From a file specified with the `MIRAKC_CONFIG` environment variable

### Build a custom Docker image

A custom Docker image for the linux/arm64 architecture can be built with the
following Dockerfile:

```Dockerfile
FROM masnagam/mirakc:arm64v8
...
```

Images for the following architectures have been uploaded to [DockerHub].

* Multi-Arch images
  * masnagam/mirakc
  * masnagam/mirakc:master
  * masnagam/mirakc:$VERSION
  * masnagam/mirakc:$PLATFORM
  * masnagam/mirakc:$VERSION-$PLATFORM
* Image for each architecture
  * masnagam/mirakc:$ARCH
  * masnagam/mirakc:master-$ARCH
  * masnagam/mirack:$VERSION-$ARCH
  * masnagam/mirack:$PLATFORM-$ARCH
  * masnagam/mirack:$VERSION-$PLATFORM-$ARCH

Where:

* PLATFORM
  * alpine
  * debian (main platform)
* ARCH
  * amd64
  * arm32v6 (linux/arm/v6 for alpine, linux/arm/v5 for debian)
  * arm32v7
  * arm64v8

Consult [dockerfile-gen](./docker/dockerfile-gen) if you like to build a Docker
image for an architecture other than the above.

### Build a Docker image for an architecture

Use `./docker/dockerfile-gen` for creating Dockerfile for each architecture:

```shell
./docker/dockerfile-gen arm64v8 >Dockerfile
docker build -t $(id -un)/mirakc:arm64v8
```

See `./docker/dockerfile-gen -h` for supported architecture.

A generated Dockerfile contains multi-stage builds for compiling the
executables.  The multi-stage builds creates untagged intermediate images like
below:

```console
$ docker images --format "{{.Repository}}:{{.Tag}}"
masnagam/mirakc:arm64v8
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
$ docker save $(id -un)/mirakc:arm64v8 | docker -H ssh://remote load
```

## Configuration

Mirakurun uses multiple files and environment variables for configuration.

For simplicity, mirakc uses a single configuration file in a YAML format like
below:

```yaml
# A property having no default value is required.

# Optional
# --------
#
# Configuration for the EPG database.
#
epg:
  # An absolute path to a folder where EPG-related data is stored.
  #
  # The default value is `None` which means no data will be saved to filesystem.
  #
  cache-dir: /path/to/epg

# Optional
# --------
#
# Configuration for the web API server.
#
server:
  # Addresses to be bound.
  # Multiple addresses can be bound.
  #
  # The default value is `http: 'localhost:40772'`.
  #
  addrs:
    - http: '0.0.0.0:40772'
    - unix: /path/to/sock

  # The number of worker threads used for serving the web API.
  #
  # The default value is a return value from `num_cpus::get()`.
  #
  workers: 4

# Required
# --------
#
# Definitions of channels.
# At least, one channel must be defined.
#
channels:
  - name: NHK E     # An arbitrary name of the channel
    type: GR        # One of channel types in GR, BS, CS and SKY
    channel: '26'   # A channel parameter used in a tuner command

  # Disable NHK G.
  # `false` by default.
  - name: NHK G
    type: GR
    channel: '27'
    disabled: true

  # Use only the service 101.
  # An empty list by default, which means that all services found are used.
  # Panics if any of specified services are not found.
  - name: BS1
    type: BS
    channel: BS15_0
    services: [101]

  # Exclude the service 531.
  # An empty list by default.
  # Applied after processing the `services` config.
  - name: OUJ
    type: BS
    channel: BS11_2
    excluded-services: [531]

# Required
# --------
#
# Definitions of tuners.
# At least, one tuner must be defined.
#
# Cascading upstream Mirakurun-compatible servers is unsupported.
#
tuners:
  - name: GR0  # An arbitrary name of the tuner

    # A list of channel types supported by the tuner.
    types: [GR]

    # A Mustache template string of a command to open the tuner.
    #
    # The command must output TS packets to STDOUT.
    #
    # Template variables:
    #
    #   channel
    #     The `channel` property of a channel defined in the `channels`.
    #
    #   channel_type
    #     The `type` property of a channel defined in the `channels`.
    #
    #   duration
    #     A duration to open the tuner in seconds.
    #     '-' means that the tuner is opened until the process terminates.
    #
    # TODO: '-' is always specified in the duration variable at this moment.
    #
    command: >-
      recdvb {{channel}} {{duration}} -

  # A tuner can be defined by using an "upstream" Mirakurun-compatible server.
  # The duration query parameter can work only for mirakc.
  - name: upstream
    types: [GR, BS]
    command: >-
      curl -sG <BASE-URL>/api/channels/{{channel_type}}/{{channel}}/stream
      -d duration={{duration}}

  - name: Disabled
    types: [GR]
    command: ''
    disabled: true  # default: false

# Optional
# --------
#
# Definitions of filters used in the TS packet pipeline like below:
#
#   tuner.command
#     | filters.pre-filter
#     | filters.[service|program]-filter
#     | filters.post-filter
#     | client
#
# Values shown below are default values.
# So, you don't need to specify any of them normally.
#
filters:
  # A Mustache template string of a command to process TS packets before a
  # packet filter program.
  #
  # The command must read TS packets from STDIN, and output the processed TS
  # packets to STDOUT.
  #
  # The defualt value is '' which means that no pre-filter program is applied
  # even when the `pre-filter=true` query parameter is specified in a streaming
  # API endpoint.
  #
  # Template variables:
  #
  #   channel
  #     The `channel` property of a channel defined in the `channels`.
  #
  #   channel_type
  #     The `type` property of a channel defined in the `channels`.
  #
  #   sid
  #     The idenfiter of the service (optional).
  #
  #   eid
  #     The event identifier of the program (optional).
  #
  pre-filter: ''

  # A Mustache template string of a command to drop TS packets which are not
  # included in a service.
  #
  # The command must read TS packets from STDIN, and output the filtered TS
  # packets to STDOUT.
  #
  # Template variables:
  #
  #   sid
  #     The idenfiter of the service.
  #
  service-filter: >-
    mirakc-arib filter-service --sid={{sid}}

  # A Mustache template string of a command to drop TS packets which are not
  # required for playback of a program in a service.
  #
  # The command must read TS packets from STDIN, and output the filtered TS
  # packets to STDOUT.
  #
  # Template variables:
  #
  #   sid
  #     The idenfiter of the service.
  #
  #   eid
  #     The event identifier of the program.
  #
  #   clock_pcr
  #     A PCR value of synchronized clock for the service.
  #
  #   clock_time
  #     A UNIX time (ms) of synchronized clock for the service.
  #
  program-filter: >-
    mirakc-arib filter-program --sid={{sid}} --eid={{eid}}
    --clock-pcr={{clock_pcr}} --clock-time={{clock_time}}
    --start-margin=5000 --end-margin=5000 --pre-streaming

  # A Mustache template string of a command to process TS packets after a packet
  # filter program.
  #
  # The command must read TS packets from STDIN, and output the processed TS
  # packets to STDOUT.
  #
  # The defualt value is '' which means that no post-filter command is applied
  # even when the `post-filter=true` query parameter is specified in a streaming
  # API endpoint.
  #
  # For compatibility with Mirakurun, the command is applied when the following
  # both conditions are met:
  #
  # * The `decode=1` query parameter is specified
  # * The `post-filter` query parameter is NOT specified
  #
  # Template variables:
  #
  #   channel
  #     The `channel` property of a channel defined in the `channels`.
  #
  #   channel_type
  #     The `type` property of a channel defined in the `channels`.
  #
  #   sid
  #     The idenfiter of the service (optional).
  #
  #   eid
  #     The event identifier of the program (optional).
  #
  post-filter: ''

# Optional
# --------
#
# Definitions for background jobs.
#
# The `command` property specifies a Mustache string.
#
# The `schedule` property specifies a crontab expression of the job schedule.
# See https://crates.io/crates/cron for details of the format.
#
# Values shown below are default values.
# So, you don't need to specify any of them normally.
#
jobs:
  # The scan-services job scans audio/video services in channels defined by the
  # `channels`.
  #
  # The command must read TS packets from STDIN, and output the result to STDOUT
  # in a specific JSON format.
  scan-services:
    command: >-
      mirakc-arib scan-services
      {{#sids}} --sids={{.}}{{/sids}}{{#xsids}} --xsids={{.}}{{/xsids}}
    schedule: '0 31 5 * * * *'  # execute at 05:31 every day

  # The sync-clocks job synchronizes TDT/TOT and PRC value of each service.
  #
  # The command must read TS packets from STDIN, and output the result to STDOUT
  # in a specific JSON format.
  sync-clocks:
    command >-
      mirakc-arib sync-clocks
      {{#sids}} --sids={{.}}{{/sids}}{{#xsids}} --xsids={{.}}{{/xsids}}
    schedule: '0 3 12 * * * *'  # execute at 12:03 every day

  # The update-schedules job updates EPG schedules for each service.
  #
  # The command must read TS packets from STDIN, and output EIT sections to
  # STDOUT in a specific line-delimited JSON format (JSONL/JSON Streaming).
  update-schedules:
    command: >-
      mirakc-arib collect-eits
      {{#sids}} --sids={{.}}{{/sids}}{{#xsids}} --xsids={{.}}{{/xsids}}
    schedule: '0 7,37 * * * * *'  # execute at 7 and 37 minutes every hour
```

## Logging

mirakc uses [log] and [env_logger] for logging.

Normally, the following configuration is used:

```shell
RUST_LOG=info
```

You can use the following configuration if you like to see more log messages for
debugging an issue:

```shell
RUST_LOG=info,mirakc=debug
```

## REST API endpoints compatible with Mirakurun

API Endpoints listed below have been implemented at this moment:

* /api/version
  * Not compatible
  * Returns only the current version string
* /api/status
  * Not compatible
  * Returns an empty object
* /api/channels
  * Compatible
  * Query parameters have **NOT** been supported
* /api/channels/{channel_type}/{channel}/stream
  * Compatible
  * The `decode` query parameter has been supported
  * The `X-Mirakurun-Priority` HTTP header has been supported
* /api/channels/{channel_type}/{channel}/services/{sid}/stream
  * Not compatible
  * Unlike Mirakurun, the `sid` must be a service ID
    * In Mirakurun, the `sid` is a service ID or an ID of the `ServiceItem`
      class
  * The `decode` query parameter has been supported
  * The `X-Mirakurun-Priority` HTTP header has been supported
* /api/services
  * Compatible
  * Query parameters have **NOT** been supported
* /api/services/{id}
  * Compatible
* /api/services/{id}/stream
  * Compatible
  * The `decode` query parameter has been supported
  * The `X-Mirakurun-Priority` HTTP header has been supported
* /api/programs
  * Compatible
  * Query parameters have **NOT** been supported
* /api/programs/{id}
  * Compatible
* /api/programs/{id}/stream
  * Compatible partially (see below)
  * The `decode` query parameter has been supported
  * The `X-Mirakurun-Priority` HTTP header has been supported
  * PSI/SI packets are sent before the program starts in order to avoid
    [issue#1313](https://github.com/actix/actix-web/issues/1313) in `actix-web`
* /api/tuners
  * Compatible
  * Query parameters have **NOT** been supported

The endpoints above are enough to run [EPGStation].

It also enough to run [BonDriver_Mirakurun].  It's strongly recommended to
enable `SERVICE_SPLIT` in `BonDriver_Mirakurun.ini` in order to reduce network
traffic between mirakc and BonDriver_Mirakurun.  Because the
`/api/channels/{channel_type}/{channel}/stream` endpoint provides a **raw** TS
stream which means that all TS packets from a tuner will be sent even though
some of them don't need for playback.

### Incompatibility of the `X-Mirakurun_Priority` header

There are the following differences of the `X-Mirakurun_Priority` header between
mirakc and Mirakurun:

* Treated as 0 when the minimum value of `X-Mirakurun-Priority` headers is less
  than 0
* Treaded as 128 when the maximum value of `X-Mirakurun-Priority` headers is
  greater than 0
* Can grab a tuner which is used by other users regardless of their priorities
  if the priority is 128

## Limitations

* `CS` and `SKY` channel types are not tested at all
  * In addition, no pay-TV channels are tested because I have no subscription
    for pay-TV
* mirakc doesn't work with BonDriver_Mirakurun at this moment
  * See the issue #4 for details

## TODO

* Use multiple tuners in the EGP task in order to reduce the time
  * Currently, it takes about 16 minutes for collecting EIT sections of 8 GR
    channels and 10 BS channels
* Support to collect logo data

## How to debug?

It's recommended to use [VS Code] for debugging.

There are two folders which contains settings regarding VS Code:

* [.devcontainer](./.devcontainer) contains settings for
  [VS Code Remote Containers]
* [.vscode](./.vscode) contains basic settings

Before starting to debug using VS Code Remote Containers, you need to create
Dockerfile with the following command:

```shell
./docker/dockerfile-gen -d amd64 >.devcontainer/Dockerfile
```

and then create `.devcontainer/config.yml`:

```shell
vi .devcontainer/config.yml
```

The following 3 configurations are defined in `.vscode/launch.json`:

* Debug
* Debug w/ child processes (Debug + log messages from child processes)
* Debug unit tests

`SIGPIPE` never stops the debugger.  See `./vscode/settings.json`.

## Notes

### mirakc leaks memory?

The memory usage of mirakc may look increasing when it runs for a long time.  If
you see this, you may suspect that mirakc is leaking memory.  But, don't worry.
mirakc does **NOT** leak memory.  In fact, the increase in the memory usage will
stop at some point.

mirakc uses system memory allocator which is default in Rust.  In many cases,
`malloc` in `glibc` is used.  The recent `malloc` in `glibc` is optimized for
multithreading.  And it doesn't free unused memory blocks in some situations.
This is the root cause of the increase in memory usage of mirakc.

There are environment variables to control criteria for freeing unused memory
blocks as described in [this page](http://man7.org/linux/man-pages/man3/mallopt.3.html).

For example, setting `MALLOC_TRIM_THRESHOLD_=-1` may improve the increase in
memory usage that occurs when accessing the `/api/programs` endpoint.

### Why use external commands to process TS packets?

Unfortunately, there is no **practical** MPEG-TS demuxer written in Rust at this
moment.

mirakc itself has no functionalities to process TS packets at all.  Therefore,
nothing can be done with mirakc alone.

For example, mirakc provides an API endpoint which returns a schedule of TV
programs, but mirakc has no functionality to collect EIT tables from TS streams.
mirakc just delegates that to an external program which is defined in the
`jobs.update-schedules.command` property in the configuration YAML file.

Of course, this design may make mirakc less efficient because using child
processes and pipes between them increases CPU and memory usages.  But it's
already been confirmed that mirakc is efficient enough than Mirakurun as you see
previously.

This design may be changed in the future if someone creates a MPEG-TS demuxer
which is functional enough for replacing the external commands.

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
[Armbian]: https://www.armbian.com/rock64/
[node-aribts]: https://www.npmjs.com/package/aribts
[px4_drv]: https://github.com/nns779/px4_drv
[Prometheus]: https://prometheus.io/
[node_exporter]: https://github.com/prometheus/node_exporter
[recdvb]: http://cgi1.plala.or.jp/~sat/?x=entry:entry180805-164428
[recpt1]: https://github.com/stz2012/recpt1
[mirakc-arib]: https://github.com/masnagam/mirakc-arib
[curl]: https://curl.haxx.se/docs/manpage.html
[socat]: http://www.dest-unreach.org/socat/doc/socat.html
[DockerHub]: https://hub.docker.com/r/masnagam/mirakc
[log]: https://crates.io/crates/log
[env_logger]: https://crates.io/crates/env_logger
[EPGStation]: https://github.com/l3tnun/EPGStation
[BonDriver_Mirakurun]: https://github.com/Chinachu/BonDriver_Mirakurun
[VS Code]: https://code.visualstudio.com/
[VS Code Remote Containers]: https://code.visualstudio.com/docs/remote/containers
[LICENSE-APACHE]: ./LICENSE-APACHE
[LICENSE-MIT]: ./LICENSE-MIT
