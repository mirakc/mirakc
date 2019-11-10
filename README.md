# mirakc

> A Mirakurun clone written in Rust

[![linux-status](https://github.com/masnagam/mirakc/workflows/Linux/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=Linux)
[![macos-status](https://github.com/masnagam/mirakc/workflows/macOS/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=macOS)
[![arm-linux-status](https://github.com/masnagam/mirakc/workflows/ARM-Linux/badge.svg)](https://github.com/masnagam/mirakc/actions?workflow=ARM-Linux)

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
running for 1 day:

```
NAME       MEM USAGE / LIMIT    MEM %   NET I/O          BLOCK I/O
mirakc     29.25MiB / 985.8MiB  2.97%   1.7MB / 1.45GB   27.1MB / 1.46GB
mirakurun  153.2MiB / 985.8MiB  15.54%  1.71MB / 1.46GB  15.2MB / 30GB
```

### 8 TS streams at the same time

mirakc is 2/3 lower CPU usage and 1/60 smaller memory consumption than
Mirakurun:

|          | mirakc@master | Mirakurun@2.11.0 |
|----------|---------------|------------------|
| CPU      | +33..38%      | +37..60%         |
| Memory   | +10..12MB     | +470MB..800MB    |
| Load1    | +1.3..4.3     | +1.5..2.7        |
| TX       | +112..120Mbps | +100..140Mbps    |
| RX       | +0.9..1.0Mbps | +0.8..1.0Mbps    |

Several hundreds or thousands of dropped packets were sometimes detected during
the performance measurement.  The same situation occurred in Mirakurun.

The performance metrics listed above were collected by using the following
command executed on a local PC:

```console
$ sh ./scripts/measure.sh <tuner-server> >/dev/null
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
mx       740098048   3936691   0
cx       1049689916  5583457   0
ex       1071480832  5699366   0
tx       1000800256  5323405   0
bs-ntv   999325696   5315562   0
bs-ex    1091991044  5808463   0
bs-tbs   1006043136  5351293   0
bs11     1412792320  7514852   0

NAME    MIN                 MAX
------  ------------------  ------------------
cpu     33.198583455832065  37.70875083500304
memory  284135424           284823552
load1   1.24                3.44
tx      113818952.48914133  120709229.84436578
rx      962611.4266622119   1012118.8965332977

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
  * Set `server.workers: 10` in /etc/mirakc/config.yml
* Tuner
  * PLEX PX-Q3U4
* Docker
  * version 19.03.1, build 74b1e89
  * Server processes were executed in a docker container

where a Prometheus server was running on the local PC.

[scripts/measure.sh](./scripts/measure.sh) performs:

* Receiving TS streams from 4 GR and 4 BS services for 10 minutes
  * `cat` is used as decoder
* Collecting system metrics by using [Prometheus] and [node_exporter]
* Counting the number of dropped TS packets by using [node-aribts]

The script should be run when the target server is idle.

You can spawn a Prometheus server using a `docker-compose.yml` in the
[docker/prometheus](./docker/prometheus) folder if you have never used it
before.  See [README.md](./docker/prometheus/README.md) in the folder before
using it.

## Configuration

Mirakurun uses multiple files and environment variables for configuration.

For simplicity, mirakc uses a single configuration file in a YAML format like
below:

```yaml
# A parameter having no default value is required.

server:
  # A IP address or hostname to bind.
  #
  # The default value is 'localhost'.
  address: '0.0.0.0'

  # A port number to bind.
  #
  # The default value is 40772.
  port: 12345

  # The number of worker threads used in the server.
  #
  # At least, the number of worker threads must be greater than:
  #
  #   Num(available local tuners) + 1
  #
  # The default value is a return value from `num_cpus::get()`.
  workers: 4

# Optional definitions of channels used for local tuners.
channels:
  - name: MX        # An arbitrary name of the channel
    type: GR        # One of channel types in GR, BS, CS and SKY
    channel: '20'   # A channel parameter used in a tuner command
    disabled: true  # default: false
  - name: CX
    type: GR
    channel: '21'
  - name: TBS
    type: GR
    channel: '22'
  - name: TX
    type: GR
    channel: '23'
  - name: EX
    type: GR
    channel: '24'
  - name: NTV
    type: GR
    channel: '25'
  - name: NHK E
    type: GR
    channel: '26'
  - name: NHK G
    type: GR
    channel: '27'
    excluded-services: [1408]  # Excludes "ＮＨＫ携帯Ｇ・東京"

# Optional definitions of local tuners.
#
# Cascading upstream Mirakurun-compatible servers is unsupported.
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
    #     The `channel` parameter of a channel defined in the `channels`.
    #
    #   duration
    #     A duration to open the tuner in seconds.
    #     '-' means that the tuner is opened until the process terminates.
    #
    command: >-
      recdvb {{channel}} {{duration}} -

  - name: Disabled
    types: [GR]
    command: ''
    disabled: true  # default: false

# Commands to process TS packets.
tools:
  # A Mustache template string of a command to scan definitions of services in
  # a TS stream.
  #
  # The command must read TS packets from STDIN, and output a JSON string to
  # STDOUT.
  #
  # Template variables:
  #
  #   xsids
  #     A list of identifiers of services which have to be excluded.
  #
  scan-services: >-
    mirakc-arib scan-services{{#xsids}} --xsid={{.}}{{/xsids}}

  # A Mustache template string of a command to collect EIT sections in a TS
  # stream.
  #
  # The command must read TS packets from STDIN, and output multiple JSON
  # strings (JSONL) to STDOUT:
  #
  # Template variables:
  #
  #   xsids
  #     A list of identifiers of services which have to be excluded.
  #
  collect-eits: >-
    mirakc-arib collect-eits{{#xsids}} --xsid={{.}}{{/xsids}}

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
  filter-service: >-
    mirakc-arib filter-packets --sid={{sid}}

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
  #   until
  #     The end datetime of the program in the UNIX timestamp represented with
  #     i64 in Rust.
  #
  filter-program: >-
    mirakc-arib filter-packets --sid={{sid}} --eid={{eid}}

  # A Mustache template string of a command to process TS packets before a
  # packet filter program.
  #
  # The command must read TS packets from STDIN, and output the processed TS
  # packets to STDOUT.
  #
  # The defualt value is '' which means that no preprocess program is applied
  # even when the `preprocess=true` query parameter is specified in a streaming
  # API endpoint.
  preprocess >-
    preprocess

  # A Mustache template string of a command to process TS packets after a packet
  # filter program.
  #
  # The command must read TS packets from STDIN, and output the processed TS
  # packets to STDOUT.
  #
  # The defualt value is '' which means that no postprocess command is applied
  # even when the `postprocess=true` query parameter is specified in a streaming
  # API endpoint.
  #
  # For compatibility with Mirakurun, the command is applied when the following
  # both conditions are met:
  #
  # * The `decode=1` query parameter is specified
  # * The `postprocess` query parameter is NOT specified
  #
  postprocess >-
    postprocess

# A string of an absolute path to a folder where EPG-related data is stored.
epg-cache-dir: /path/to/epg
```

## Logging

mirakc uses [log] and [env_logger] for logging.

Normally, the following configuration is used:

```
RUST_LOG=info
```

You can use the following configuration if you like to see more log messages for
debugging an issue:

```
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
  * The optional `duration` query parameter (in milliseconads) has been added,
    which is passed to the tuner command
* /api/services
  * Compatible
  * Query parameters have **NOT** been supported
* /api/services/{id}/stream
  * Compatible
  * The `decode` query parameter has been supported
  * The `X-Mirakurun-Priority` HTTP header has been supported
  * The optional `duration` query parameter (in milliseconads) has been added,
    which is passed to the tuner command
* /api/programs
  * Compatible
  * Query parameters have **NOT** been supported
* /api/programs/{id}/stream
  * Compatible
  * The `decode` query parameter has been supported
  * The `X-Mirakurun-Priority` HTTP header has been supported
  * The optional `duration` query parameter (in milliseconads) has been added,
    which is passed to the tuner command
* /api/tuners
  * Compatible
  * Query parameters have **NOT** been supported

The endpoints above are required for working with [EPGStation].

## Limitations

* `CS` and `SKY` channel types are not tested at all
  * In addition, no pay-TV channels are tested because I have no subscription
    for pay-TV
* A tuner can be used by a single user exclusively
  * Mirakurun allows to share a tuner with multiple users and deliver TS packets
    to the users concurrently
* Reading TS packets from a tuner blocks a thread
  * That is the reason why the `server.workers` configuration must be greater
    than the total number of local tuners

## TODO

* Use multiple tuners in the EGP task in order to reduce the time
  * Currently, it takes about 16 minutes for collecting EIT sections of 8 GR
    channels and 10 BS channels
* Support to collect logo data

## mirakc leaks memory?

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

For example, setting `MALLOC_TRIM_THRESHOLD_=-1` improves the increase in memory
usage that occurs when accessing the `/api/programs` endpoint.

## Why use external commands to process TS packets?

Unfortunately, there is no **practical** MPEG-TS demuxer written in Rust at this
moment.

mirakc itself has no functionalities to process TS packets at all.  Therefore,
nothing can be done with mirakc alone.

For example, mirakc provides an API endpoint which returns a schedule of TV
programs, but mirakc has no functionality to collect EIT tables from TS streams.
mirakc just delegates that to an external program which is defined in the
`tools.collect-eits` property in the configuration YAML file.

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
[log]: https://crates.io/crates/log
[env_logger]: https://crates.io/crates/env_logger
[EPGStation]: https://github.com/l3tnun/EPGStation
[LICENSE-APACHE]: ./LICENSE-APACHE
[LICENSE-MIT]: ./LICENSE-MIT
