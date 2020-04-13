# Configuration

For simplicity, mirakc uses a single YAML file for configuration.

Most of properties in the configuration are optional.  You can simply omit
properties which have default values listed below if the default values are
suitable for your environment.

| PROPERTY                         | DEFAULT                                   |
|----------------------------------|-------------------------------------------|
| [epg.cache-dir]                  | `None`                                    |
| [server.addrs]                   | `[{http: 'localhost:40772'}]`             |
| [server.workers]                 | The number of CPUs                        |
| [server.stream-chunk-size]       | `32768` (32KiB)                           |
| [server.stream-max-chunks]       | `1000`                                    |
| [channels\[\].name]              |                                           |
| [channels\[\].type]              |                                           |
| [channels\[\].channel]           |                                           |
| [channels\[\].services]          | `[]`                                      |
| [channels\[\].excluded-services] | `[]`                                      |
| [channels\[\].disabled]          | `false`                                   |
| [tuners\[\].name]                |                                           |
| [tuners\[\].types]               |                                           |
| [tuners\[\].command]             |                                           |
| [tuners\[\].time-limit]          | `30000` (30s)                             |
| [tuners\[\].disabled]            | `false`                                   |
| [filters.pre-filter]             | `''`                                      |
| [filters.service-filter]         | `mirakc-arib filter-service --sid={{sid}}`|
| [filters.program-filter]         | `mirakc-arib filter-program --sid={{sid}} --eid={{eid}} --clock-pcr={{clock_pcr}} --clock-time={{clock_time}} --start-margin=5000 --end-margin=5000 --pre-streaming` |
| [filters.post-filter]            | `''`                                      |
| [jobs.scan-services.command]     | `mirakc-arib scan-services{{#sids}} --sids={{.}}{{/sids}}{{#xsids}} --xsids={{.}}{{/xsids}}` |
| [jobs.scan-services.schedule]    | `'0 31 5 * * * *'` (execute at 05:31 every day) |
| [jobs.sync-clocks.command]       | `mirakc-arib sync-clocks{{#sids}} --sids={{.}}{{/sids}}{{#xsids}} --xsids={{.}}{{/xsids}}` |
| [jobs.sync-clocks.schedule]      | `'0 3 12 * * * *'` (execute at 12:03 every day) |
| [jobs.update-schedules.command]  | `mirakc-arib collect-eits{{#sids}} --sids={{.}}{{/sids}}{{#xsids}} --xsids={{.}}{{/xsids}}` |
| [jobs.update-schedules.schedule] | `'0 7,37 * * * * *'` (execute at 7 and 37 minutes every hour) |
| [mirakurun.openapi-json]         | `/etc/mirakurun.openapi.json`             |

[epg.cache-dir]: #epg.cache-dir
[server.addrs]: #server.addrs
[server.workers]: #server.workers
[server.stream-chunk-size]: #server.stream-chunk-size
[server.stream-max-chunks]: #server.stream-max-chunks
[channels\[\].name]: #channels
[channels\[\].type]: #channels
[channels\[\].channel]: #channels
[channels\[\].services]: #channels
[channels\[\].excluded-services]: #channels
[channels\[\].disabled]: #channels
[tuners\[\].name]: #tuners
[tuners\[\].types]: #tuners
[tuners\[\].command]: #tuners
[tuners\[\].time-limit]: #tuners
[tuners\[\].disabled]: #tuners
[filters.pre-filter]: #filters.pre-filter
[filters.service-filter]: #filters.service-filter
[filters.program-filter]: #filters.program-filter
[filters.post-filter]: #filters.post-filter
[jobs.scan-services.command]: #jobs.scan-services.command
[jobs.scan-services.schedule]: #jobs.scan-services.schedule
[jobs.sync-clocks.command]: #jobs.sync-clocks.command
[jobs.sync-clocks.schedule]: #jobs.sync-clocks.schedule
[jobs.update-schedules.command]: #jobs.update-schedules.command
[jobs.update-schedules.schedule]: #jobs.update-schdules.schedule
[mirakurun.openapi-json]: #mirakurun.openapi-json

## epg.cache-dir

An absolute path to a folder where EPG-related data is stored.

`None` means that no data will be saved onto the filesystem.  In this case,
mirakc always invokes background jobs at startup.

```yaml
epg:
  cache-dir: /path/to/epg/cache
```

## server.addrs

`server.addrs` is a list of addresses to be bound.

There are two address types.

HTTP protocol:

```yaml
server:
  addrs:
    - http: '0.0.0.0:40772'
```

HTTPS protocol is not supported at this point.

UNIX domain socket:

```yaml
server:
  addrs:
    - unix: /var/run/mirakc.sock
```

mirakc never changes the ownership and permission of the socket.  Change them
after the socket has been created.  Or use SUID/SGID so that mirakc runs with
specific UID/GID.

Multiple addresses can be bound like below:

```yaml
server:
  addrs:
    - http: '0.0.0.0:40772'
    - unix: /var/run/mirakc.sock
```

## server.workers

The number of worker threads to serve the web API.

The specified number of threads are spawned and pooled at startup.

```yaml
server:
  workers: 2
```

## server.stream-chunk-size

The maximum size of a chunk for streaming.

```yaml
server:
  stream-chunk-size: 32768
```

An actual size of a chunk may be smaller than this value.

The default chunk size is 32 KiB which is large enough for 10 ms buffering.

## server.stream-max-chunks

The maximum number of chunks that can be buffered.

```yaml
server:
  stream-max-chunks: 1000
```

Chunks are dropped when the buffer is full.

The default maximum number of chunks is 1000 which is large enough for 10
seconds buffering if the chunk size is 32 KiB.

## channels

Definitions of channels.  At least, one channel must be defined.

* name
  * An arbitrary name of the channel
* type
  * One of channel types in `GR`, `BS`, `CS` and `SKY`
* channel
  * A channel parameter used in a tuner command template
* services
  * A list of SIDs (service identifiers) which must be included
  * An empty list means that all services found are included
* excluded-services
  * A list of SIDs which must be excluded
  * An empty list means that no service is excluded
  * Applied after processing the `services` property
* disabled (optional)
  * Disable the channel definition

```yaml
channels:
  - name: ETV
    type: GR
    channel: '26'

  # Disable NHK.
  - name: NHK
    type: GR
    channel: '27'
    disabled: true

  # Use only the service 101 in BS1.
  - name: BS1
    type: BS
    channel: BS15_0
    services: [101]

  # Exclude the service 531 from OUJ.
  - name: OUJ
    type: BS
    channel: BS11_2
    excluded-services: [531]
```

## tuners

Definitions of tuners.  At least, one tuner must be defined.

* name
  * An arbitrary name of the tuner
* types
  * A list of channel types supported by the tuner.
* command
  * A Mustache template string of a command to open the tuner
  * The command must output TS packets to `stdout`
* time-limit (optional)
  * A time limit in milliseconds
  * Stop streaming if no TS packet comes from the tuner for the time limit
* disabled (optional)
  * Disable the tuner

Command template variables:

* channel
  * The `channel` property of a channel defined in the `channels`
* channel_type
  * The `type` property of a channel defined in the `channels`
* duration
  * A duration to open the tuner in seconds
  * `-` means that the tuner is opened until the process terminates
  * TODO: `-` is always specified in the duration variable at this moment

Cascading upstream Mirakurun-compatible servers is unsupported.  However, it's
possible to use upstream Mirakurun-compatible servers as tuners.  See the sample
below.

```yaml
tuners:
  - name: GR0
    types: [GR]
    command: recdvb {{channel}} {{duration}} -

  - name: Disabled
    types: [GR]
    command: cat /dev/null
    disabled: true

  # A tuner can be defined by using an "upstream" Mirakurun-compatible server.
  - name: upstream
    types: [GR, BS]
    command: >-
      curl -sG http://upstream:40772/api/channels/{{channel_type}}/{{channel}}/stream

```

## filters

Definitions of filters used in
[the streaming pipeline](./inside-mirakc.md#streaming-pipeline).

### filters.pre-filter

A Mustache template string of a command to process TS packets before a packet
filter program.

The command must read TS packets from `stdin`, and output the processed TS
packets to `stdout`.

An empty string means that no pre-filter program is applied even when the
`pre-filter=true` query parameter is specified in a streaming API endpoint.

Template variables:

* channel
  * The `channel` property of a channel defined in the `channels`
* channel_type
  * The `type` property of a channel defined in the `channels`
* sid (optional)
  * The idenfiter of the service
* eid (optional)
  * The event identifier of the program

### filters.service-filter

A Mustache template string of a command to drop TS packets which are not
included in a service.

The command must read TS packets from `stdin`, and output the filtered TS
packets to `stdout`.

Template variables:

* sid
  * The idenfiter of the service

### filters.program-filter

A Mustache template string of a command to drop TS packets which are not
required for playback of a program in a service.

The command must read TS packets from `stdin`, and output the filtered TS
packets to `stdout`.

Template variables:

* sid
  * The idenfiter of the service
* eid
  * The event identifier of the program
* clock_pcr
  * A PCR value of synchronized clock for the service
* clock_time
  * A UNIX time (ms) of synchronized clock for the service

### filters.post-filter

A Mustache template string of a command to process TS packets after a packet
filter program.

The command must read TS packets from `stdin`, and output the processed TS
packets to `stdout`.

An empty string means that no post-filter command is applied even when the
`post-filter=true` query parameter is specified in a streaming API endpoint.

For compatibility with Mirakurun, the command is applied when the following both
conditions are met:

* The `decode=1` query parameter is specified
* The `post-filter` query parameter is NOT specified

Template variables:

* channel
  * The `channel` property of a channel defined in the `channels`
* channel_type
  * The `type` property of a channel defined in the `channels`
* sid (optional)
  * The idenfiter of the service
* eid (optional)
  * The event identifier of the program

## jobs

Definitions for background jobs.

Each job definition has the following two properties:

* command
  * A Mustache template string of a command
* schedule
  * A crontab expression of the job schedule
  * See https://crates.io/crates/cron for details of the format

### jobs.scan-services

The scan-services job scans audio/video services in channels defined in the
`channels`.

The command must read TS packets from `stdin`, and output the result to `stdout`
in a specific JSON format.  See the help shown by `mirakc-arib scan-services -h`
for details of the JSON format.

Command template variables:

* sids
  * A list of SIDs which must be included
* xsids
  * A list of SIDs which must be excluded

### jobs.sync-clocks

The sync-clocks job synchronizes TDT/TOT and PRC value of each service.

The command must read TS packets from `stdin`, and output the result to `stdout`
in a specific JSON format.  See the help shown by `mirakc-arib sync-clocks -h`
for details of the JSON format.

Command template variables:

* sids
  * A list of SIDs which must be included
* xsids
  * A list of SIDs which must be excluded

### jobs.update-schedules

The update-schedules job updates EPG schedules for each service.

The command must read TS packets from `stdin`, and output the result to `stdout`
in a specific JSON format.  See the help shown by `mirakc-arib collect-eits -h`
for details of the JSON format.

Command template variables:

* sids
  * A list of SIDs which must be included
* xsids
  * A list of SIDs which must be excluded

## mirakurun.openapi-json

`mirakurun.openapi-json` specifies a path to an OpenAPI/Swagger JSON file
obtained from Mirakurun.

Applications including EPGStation use the Mirakurun client which uses `api/docs`
in order to query interfaces implemented by a web server that the client will
communicate.

```yaml
mirakurun:
  openapi-json: /path/to/mirakurun.openapi.json
```
