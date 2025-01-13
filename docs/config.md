# Configuration

For simplicity, mirakc uses a single YAML/TOML file for configuration.

Most of properties in the configuration are optional.  You can simply omit
properties which have default values listed below if the default values are
suitable for your environment.

| PROPERTY                                 | DEFAULT                           |
|------------------------------------------|-----------------------------------|
| [epg.cache-dir]                          | `None`                            |
| [server.addrs]                           | `[{http: 'localhost:40772'}]`     |
| [server.stream-chunk-size]               | `32768` (32KiB)                   |
| [server.stream-max-chunks]               | `1000`                            |
| [server.stream-time-limit]               | `16000` (16s)                     |
| [server.program-stream-max-start-delay]  | `None`                            |
| [server.mounts]                          | `{}`                              |
| [server.folder-view-template-path]       | `None`                            |
| [channels\[\].name]                      |                                   |
| [channels\[\].type]                      |                                   |
| [channels\[\].channel]                   |                                   |
| [channels\[\].extra-args]                | `''`                              |
| [channels\[\].services]                  | `[]`                              |
| [channels\[\].excluded-services]         | `[]`                              |
| [channels\[\].disabled]                  | `false`                           |
| [tuners\[\].name]                        |                                   |
| [tuners\[\].types]                       |                                   |
| [tuners\[\].command]                     |                                   |
| [tuners\[\].time-limit]                  | `30000` (30s)                     |
| [tuners\[\].disabled]                    | `false`                           |
| [tuners\[\].decoded]                     | `false`                           |
| [tuners\[\].excluded-channels]           | `[]`                              |
| [filters.tuner-filter.command]           | `''`                              |
| [filters.service-filter.command]         | `mirakc-arib filter-service --sid={{{sid}}}` |
| [filters.decode-filter.command]          | `''`                              |
| [filters.program-filter.command]         | `mirakc-arib filter-program --sid={{{sid}}} --eid={{{eid}}} --clock-pid={{{clock_pid}}} --clock-pcr={{{clock_pcr}}} --clock-time={{{clock_time}}} --end-margin=2000{{#video_tags}} --video-tag={{{.}}}{{/video_tags}}{{#audio_tags}} --audio-tag={{{.}}}{{/audio_tags}}{{#if wait_until}} --wait-until={{{wait_until}}}{{/if}}` |
| [pre-filters.\*.command]                 | `''`                              |
| [pre-filters.\*.seekable]                | `false`                           |
| [post-filters.\*.command]                | `''`                              |
| [post-filters.\*.content-type]           | `None`                            |
| [post-filters.\*.seekable]               | `false`                           |
| [jobs.scan-services.command]             | `timeout 30 mirakc-arib scan-services{{#sids}} --sids={{{.}}}{{/sids}}{{#xsids}} --xsids={{{.}}}{{/xsids}}` (timeout: 30s) |
| [jobs.scan-services.schedule]            | `'0 1 8,20 * * * *'` (execute at 08:01 and 20:01 every day) |
| [jobs.scan-services.disabled]            | `false`                           |
| [jobs.sync-clocks.command]               | `timeout 30 mirakc-arib sync-clocks{{#sids}} --sids={{{.}}}{{/sids}}{{#xsids}} --xsids={{{.}}}{{/xsids}}` (timeout: 30s) |
| [jobs.sync-clocks.schedule]              | `'0 11 8,20 * * * *'` (execute at 08:11 and 20:11 every day) |
| [jobs.sync-clocks.disabled]              | `false`                           |
| [jobs.update-schedules.command]          | `timeout 600 mirakc-arib collect-eits{{#sids}} --sids={{{.}}}{{/sids}}{{#xsids}} --xsids={{{.}}}{{/xsids}}` (timeout: 10m) |
| [jobs.update-schedules.schedule]         | `'0 21 8,20 * * * *'` (execute at 08:21 and 20:21 every day) |
| [jobs.update-schedules.disabled]         | `false`                           |
| [recording.basedir]                      | `None`                            |
| [recording.records-dir]                  | `None`                            |
| [timeshift.command]                      | `'mirakc-arib record-service --sid={{{sid}}} --file={{{file}}} --chunk-size={{{chunk_size}}} --num-chunks={{{num_chunks}}} --start-pos={{{start_pos}}}'` |
| [timeshift.recorders\[\].service-id]     |                                   |
| [timeshift.recorders\[\].ts-file]        |                                   |
| [timeshift.recorders\[\].data-file]      |                                   |
| [timeshift.recorders\[\].chunk-size]     | `154009600` (~154MB)              |
| [timeshift.recorders\[\].num-chunks]     |                                   |
| [timeshift.recorders\[\].num-reserves]   | `1`                               |
| [timeshift.recorders\[\].priority]       | `128`                             |
| [timeshift.recorders\[\].uses.tuner]     |                                   |
| [timeshift.recorders\[\].uses.channel-type]|                                 |
| [timeshift.recorders\[\].uses.channel]   |                                   |
| [onair-program-trackers]                 | `{}`                              |
| [resource.strings-yaml]                  | `/etc/mirakc/strings.yml`         |
| [resource.logos]                         | `[]`                              |

[epg.cache-dir]: #epgcache-dir
[server.addrs]: #serveraddrs
[server.stream-chunk-size]: #serverstream-chunk-size
[server.stream-max-chunks]: #serverstream-max-chunks
[server.stream-time-limit]: #serverstream-time-limit
[server.program-stream-max-start-delay]: #serverprogram-stream-max-start-delay
[server.mounts]: #servermounts
[server.folder-view-template-path]: #serverfolder-view-template-path
[channels\[\].name]: #channels
[channels\[\].type]: #channels
[channels\[\].channel]: #channels
[channels\[\].extra-args]: #channels
[channels\[\].services]: #channels
[channels\[\].excluded-services]: #channels
[channels\[\].disabled]: #channels
[tuners\[\].name]: #tuners
[tuners\[\].types]: #tuners
[tuners\[\].command]: #tuners
[tuners\[\].time-limit]: #tuners
[tuners\[\].disabled]: #tuners
[tuners\[\].decoded]: #tuners
[tuners\[\].excluded-channels]: #tuners
[filters.tuner-filter.command]: #filterstuner-filter
[filters.service-filter.command]: #filtersservice-filter
[filters.decode-filter.command]: #filtersdecode-filter
[filters.program-filter.command]: #filtersprogram-filter
[pre-filters.\*.command]: #pre-filters
[pre-filters.\*.seekable]: #pre-filters
[post-filters.\*.command]: #post-filters
[post-filters.\*.content-type]: #post-filters
[post-filters.\*.seekable]: #post-filters
[jobs.scan-services.command]: #jobsscan-services
[jobs.scan-services.schedule]: #jobsscan-services
[jobs.scan-services.disabled]: #jobsscan-services
[jobs.sync-clocks.command]: #jobssync-clocks
[jobs.sync-clocks.schedule]: #jobssync-clocks
[jobs.sync-clocks.disabled]: #jobssync-clocks
[jobs.update-schedules.command]: #jobsupdate-schedules
[jobs.update-schedules.schedule]: #jobsupdate-schedules
[jobs.update-schedules.disabled]: #jobsupdate-schedules
[recording.basedir]: #recordingbasedir
[recording.records-dir]: #recordingrecords-dir
[timeshift.command]: #timeshift
[timeshift.recorders\[\].service-id]: #timeshiftrecorders
[timeshift.recorders\[\].ts-file]: #timeshiftrecorders
[timeshift.recorders\[\].data-file]: #timeshiftrecorders
[timeshift.recorders\[\].chunk-size]: #timeshiftrecorders
[timeshift.recorders\[\].num-chunks]: #timeshiftrecorders
[timeshift.recorders\[\].num-reserves]: #timeshiftrecorders
[timeshift.recorders\[\].priority]: #timeshiftrecorders
[timeshift.recorders\[\].uses.tuner]: #timeshiftrecorders
[timeshift.recorders\[\].uses.channel-type]: #timeshiftrecorders
[timeshift.recorders\[\].uses.channel]: #timeshiftrecorders
[onair-program-trackers]: #onair-program-trackers
[resource.strings-yaml]: #resourcestrings-yaml
[resource.logos]: #resourcelogos

## epg.cache-dir

An absolute path to a folder where EPG-related data will be stored.

`None` means that no data will be saved onto the filesystem.  In this case,
EPG-related data will be lost when mirakc stops.

```yaml
# YAML
epg:
  cache-dir: /path/to/epg/cache
```

```toml
# TOML
[epg]
cache-dir = "/path/to/epg/cache"
```

## server.addrs

`server.addrs` is a list of addresses to be bound.

There are two address types.

HTTP protocol:

```yaml
# YAML
server:
  addrs:
    - http: '0.0.0.0:40772'
```

```toml
# TOML
[[server.addrs]]
http = "0.0.0.0:40772"
```

HTTPS protocol is not supported at this point.

UNIX domain socket:

```yaml
# YAML
server:
  addrs:
    - unix: /var/run/mirakc.sock
```

```toml
# TOML
[[server.addrs]]
unix = "/var/run/mirakc.sock"
```

mirakc never changes the ownership and permission of the socket.  Change them
after the socket has been created.  Or use SUID/SGID so that mirakc runs with
specific UID/GID.

Multiple addresses can be bound like below:

```yaml
# YAML
server:
  addrs:
    - http: '0.0.0.0:40772'
    - unix: /var/run/mirakc.sock
```

```toml
# TOML
[[server.addrs]]
http = "0.0.0.0:40772"

[[server.addrs]]
unix = "/var/run/mirakc.sock"
```

## server.stream-chunk-size

The maximum size of a chunk for streaming.

```yaml
# YAML
server:
  stream-chunk-size: 32768
```

```toml
# TOML
[server]
stream-chunk-size = 32768
```

An actual size of a chunk may be smaller than this value.

The default chunk size is 32 KiB which is large enough for 10 ms buffering.

## server.stream-max-chunks

The maximum number of chunks that can be buffered.

```yaml
# YAML
server:
  stream-max-chunks: 1000
```

```toml
# TOML
[server]
stream-max-chunks = 1000
```

Chunks are dropped when the buffer is full.

The default maximum number of chunks is 1000 which is large enough for 10
seconds buffering if the chunk size is 32 KiB.

## server.stream-time-limit

The time limit for a streaming request.  The request will fail when the time
reached this limit.

```yaml
# YAML
server:
  stream-time-limit: 20000
```

```toml
# TOML
[server]
stream-time-limit = 20000
```

The value must be larger than `prepTime` defined in [EPGStation](https://github.com/l3tnun/EPGStation/blob/v1.6.9/src/server/Model/Operator/Recording/RecordingManageModel.ts#L45),
which is `15000` (15s) in v1.6.9.

### Historical Notes

This property is needed for avoiding the issue#1313 in actix-web in a streaming
request for a TV program.  In this case, no data is sent to the client until the
first TS packet comes from the streaming pipeline.  actix-web cannot detect the
client disconnect all that time due to the issue#1313.

### server.program-stream-max-start-delay

`program-stream-max-start-delay` can be used to specify a maximum delay for the start
time of a TV program in [a human-friendly format](https://github.com/tailhook/humantime).

```yaml
# YAML
server:
  program-stream-max-start-delay: 3h
```

```toml
# TOML
[server]
program-stream-max-start-delay = "3h"
```

The value must be less than `24h`.  The value will be converted into the number
of whole seconds and its fractional part will be ignored.

Streaming with `mirakc-arib filter-program` will wait the start of a TV program
for an amount of time specified with `program-stream-max-start-delay` if it's
specified.  In the meantime, `mirakc-arib filter-program` keeps a tuner device
open.

`program-stream-max-start-delay` is disabled by default.

## server.mounts

Definitions of mount points for static files and folders on the file system.

* path
  * An absolute path to an existing directory on the file system
* index (default: `None`)
  * A name of the index file
* listing (default: `false`)
  * Show entries listing for directories

```yaml
# YAML
server:
  mounts:
    /public:
      path: /path/to/public
      listing: true
    /www:
      path: /path/to/www
      index: index.html
```

```toml
# TOML
[server.mounts."/public"]
path = "/path/to/public"
listing = true

[server.mounts."/www"]
path = "/path/to/www"
index = "index.html"
```

This property can be used for providing some kind of Web UI for mirakc.

## server.folder-view-template-path

A path to a Mustache template file rendered to an HTML document for the folder
view of static files mounted by using `server.mounts`.

The filename must end with the `.mustache` extension.

```yaml
# YAML
server:
  folder-view-template-path: /path/to/template.html.mustache
```

```toml
# TOML
[server]
folder-view-template-path = "/path/to/template.html.mustache"
```

Template variables:

* entries
  * name
    * The name of the entry
  * size
    * The size of the entry
  * url
    * The URL of the entry
  * created
    * The creation time of the entry in UNIX time
  * modified
    * The last modification time of the entry in UNIX time

Example:

```html
<div>
  {{#entries}}
  {{name}},{{size}},{{url}},{{created}},{{modified}}
  {{/entries}}
</div>
```

The built-in template is used by default.

## channels

Definitions of channels.  At least, one channel must be defined.

* name
  * An arbitrary name of the channel
* type
  * One of channel types in `GR`, `BS`, `CS`, `SKY` and `BS4K`
* channel
  * A channel parameter used in a tuner command template
* extra-args
  * Extra arguments used in a tuner command template
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
# YAML
channels:
  - name: ETV
    type: GR
    channel: '26'

  # Disable NHK.
  - name: NHK
    type: GR
    channel: '27'
    disabled: true

  # Use only the service 101 in NHK-BS.
  - name: NHK-BS
    type: BS
    channel: BS15_0
    services: [101]

  # Exclude the service 531 from OUJ.
  - name: OUJ
    type: BS
    channel: BS11_2
    excluded-services: [531]

  # Extra arguments for szap-s2j
  - name: BS SPTV
    type: SKY
    channel: CH585
    extra-args: '-l JCSAT3A'
    serviceId: [33353]

  # Extra arguments for BonRecTest
  - name: ND02
    type: CS
    channel: '000'
    extra-args: '--space 1'
```

```toml
# TOML
[[channels]]
name = "ETV"
type = "GR"
channel = "26"

# Disable NHK.
[[channels]]
name = "NHK"
type = "GR"
channel = "27"
disabled = true

# Use only the service 101 in NHK-BS.
[[channels]]
name = "NHK-BS"
type = "BS"
channel = "BS15_0"
services = [ 101 ]

# Exclude the service 531 from OUJ.
[[channels]]
name = "OUJ"
type = "BS"
channel = "BS11_2"
excluded-services = [ 531 ]

# Extra arguments for szap-s2j
[[channels]]
name = "BS SPTV"
type = "SKY"
channel = "CH585"
extra-args = "-l JCSAT3A"
serviceId = [ 33353 ]

# Extra arguments for BonRecTest
[[channels]]
name = "ND02"
type = "CS"
channel = "000"
extra-args = "--space 1"
```

Definitions with the same `type` and `channel` will be merged.  For example, the
following definitions:

```yaml
# YAML
channels:
  - name: NHK1
    type: GR
    channel: '27'
    services: [1024]
  - name: NHK2
    type: GR
    channel: '27'
    services: [1025]

  - name: ETV
    type: GR
    channel: '26'
    excluded-services: [1034]
  - name: ETV3
    type: GR
    channel: '26'
    services: [1034]

  - name: NHK-BS
    type: BS
    channel: BS15_0
    extra-args: args
  - name: NHK-BS
    type: BS
    channel: BS15_0
    extra-args: different-args
```

```toml
# TOML
[[channels]]
name = "NHK1"
type = "GR"
channel = "27"
services = [ 1024 ]

[[channels]]
name = "NHK2"
type = "GR"
channel = "27"
services = [ 1025 ]

[[channels]]
name = "ETV"
type = "GR"
channel = "26"
excluded-services = [ 1034 ]

[[channels]]
name = "ETV3"
type = "GR"
channel = "26"
services = [ 1034 ]

[[channels]]
name = "NHK-BS"
type = "BS"
channel = "BS15_0"
extra-args = "args"

[[channels]]
name = "BS1"
type = "BS"
channel = "BS15_0"
extra-args = "different-args"
```

are equivalent to:

```yaml
# YAML
channels:
  # `services` of channels having the same `type` and `channel` will be merged.
  - name: ETV
    type: GR
    channel: '27'
    services: [1024, 1025]

  # `services` becomes empty if there is a channel with empty `services`.
  - name: ETV
    type: GR
    channel: '27'
    excluded-services: [1034]

  # Channels having the same `type` and `channel` should have the same
  # `extra-args`.
  - name: HNK-BS
    type: BS
    channel: BS15_0
    extra-args: args
```

```toml
# TOML
[[channels]]
# `services` of channels having the same `type` and `channel` will be merged.
name = "ETV"
type = "GR"
channel = "27"
services = [ 1024, 1025 ]

[[channels]]
# `services` becomes empty if there is a channel with empty `services`.
name = "ETV"
type = "GR"
channel = "27"
excluded-services = [ 1034 ]

[[channels]]
# Channels having the same `type` and `channel` should have the same
# `extra-args`.
name = "HNK-BS"
type = "BS"
channel = "BS15_0"
extra-args = "args"
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
* decoded (optional)
  * PES packets are decoded by the tuner command
* excluded-channels (optional)
  * A list of excluded channels

Command template variables:

* channel
  * The `channel` property of a channel defined in the `channels`
* channel_type
  * The `type` property of a channel defined in the `channels`
* duration
  * A duration to open the tuner in seconds
  * `-` means that the tuner is opened until the process terminates
  * TODO: `-` is always specified in the duration variable at this moment
* extra_args
  * The `extra-args` property of a channel defined in the `channels`

Cascading upstream Mirakurun-compatible servers is unsupported.  However, it's
possible to use upstream Mirakurun-compatible servers as tuners.  See the sample
below.

```yaml
# YAML
tuners:
  - name: GR0
    types: [GR]
    command: >-
      recdvb {{{channel}}} {{{duration}}} -

  - name: Disabled
    types: [GR]
    command: >-
      cat /dev/null
    disabled: true

  # A tuner can be defined by using an "upstream" Mirakurun-compatible server.
  - name: upstream
    types: [GR, BS]
    command: >-
      curl -sG http://upstream:40772/api/channels/{{{channel_type}}}/{{{channel}}}/stream?decode=0

  # Exclude a particular channel by name if channel names defined in `channels` are unique.
  - name: exclude-channel-by-name
    types: [GR]
    command: ...
    excluded-channels:
      - name: excluded-channel-name

  # Exclude a particular channel by channel parameters.
  - name: exclude-channel-by-params
    types: [GR]
    command: ...
    excluded-channels:
      - params:
          channel-type: GR
          channel: exclude-channel
```

```toml
# TOML
[[tuners]]
name = "GR0"
types = [ "GR" ]
command = "recdvb {{{channel}}} {{{duration}}} -"

[[tuners]]
name = "Disabled"
types = [ "GR" ]
command = "cat /dev/null"
disabled = true

# A tuner can be defined by using an "upstream" Mirakurun-compatible server.
[[tuners]]
name = "upstream"
types = [ "GR", "BS" ]
command = "curl -sG http://upstream:40772/api/channels/{{{channel_type}}}/{{{channel}}}/stream?decode=0"
```

## filters

Definitions of filters used in
[the streaming pipeline](./inside-mirakc.md#streaming-pipeline).

The following properties can be specified in `config.yml`:

* command
  * A Mustache template string of the filter command
  * The command must read data from `stdin`, and output the processed data to
    `stdout`
  * An empty string means that the filter is not defined
* content-type
  * A string of the content-type of data output from the filter
  * Absence of this property means that the filter doesn't change the
    content-type of the input data
  * Available only for the `post-filters`
* seekable (default: `false`)
  * `true`: Range requests are acceptable when the filter is used
  * `false`: Range requests are **NOT** acceptable when the filter is used

Each filter has the following properties:

| PROPERTY     | tuner-filter | decode-filter | service-filter | program-filter | pre-filter | post-filter |
| ------------ | ------------ | ------------- | -------------- | -------------- | ---------- | ----------- |
| command      | `?`          | `?`           | `?`            | `?`            | `?`        | `?`         |
| seekable     | `false`      | `false`       | `false`        | `false`        | `?`        | `?`         |
| content-type |              |               |                |                |            | `?`         |

Where:

* `?` means that the property is configurable in `config.yml`
* Empty cell means that the property is **not** available for the filter

Each Mustache template string defined in the `command` property will be rendered
with the following template parameters:

* tuner_index
  * The index of a tuner
* tuner_name
  * The `name` property of a tuner defined in the `tuners`
* channel_name
  * The `name` property of a channel defined in the `channels`
* channel_type
  * The `type` property of a channel defined in the `channels`
* channel
  * The `channel` property of a channel defined in the `channels`
* user
  * Contains information about a user of the stream
  * See `test_make_filter()` in [//mirakc-core/src/filter.rs](../mirakc-core/src/filter.rs)
    about how to use it
* sid
  * The 16-bit integer identifier of a service (SID)
  * Available only for the service streaming, the program streaming and the record streaming
* eid
  * The 16-bit integer identifier of a program (EID)
  * Available only for the program streaming and the record streaming
* clock_pid
  * A PID of PCR packets to be used for the clock synchronization
  * Available only for the program streaming
* clock_pcr
  * A PCR value of synchronized clock for a service
  * Available only for the program streaming
* clock_time
  * A UNIX time (ms) of synchronized clock for a service
  * Available only for the program streaming
* video_tags
  * `component_tag` of a video stream in the program
  * Available only for the program streaming and the record streaming
* audio_tags
  * `component_tag`s of audio streams in the program
  * Available only for the program streaming and the record streaming
* id
  * The identifier of a record
  * Available only for the record streaming
* duration
  * A duration of a record in seconds
  * Available only for the record streaming
* size
  * Size of a record in bytes
  * Available only for the record streaming

Each filter has the following template parameter:

| PARAMETER    | tuner-filter | decode-filter | service-filter | program-filter | pre-filter | post-filter |
| ------------ | ------------ | ------------- | -------------- | -------------- | ---------- | ----------- |
| tuner_index  | `*`          |               |                |                |            |             |
| tuner_name   | `*`          |               |                |                |            |             |
| channel_name | `*`          | `*`           | `*`            | `*`            | `*`        | `*`         |
| channel_type | `*`          | `*`           | `*`            | `*`            | `*`        | `*`         |
| channel      | `*`          | `*`           | `*`            | `*`            | `*`        | `*`         |
| user         |              |               | `*`            | `*`            | `SP`       | `SP`        |
| sid          |              |               | `*`            | `*`            | `SPRTL`    | `SPRTL`     |
| eid          |              |               |                | `*`            | `PRT`      | `PRT`       |
| clock_pid    |              |               |                | `*`            | `P`        | `P`         |
| clock_pcr    |              |               |                | `*`            | `P`        | `P`         |
| clock_time   |              |               |                | `*`            | `P`        | `P`         |
| video_tags   |              |               |                | `*`            | `PRT`      | `PRT`       |
| audio_tags   |              |               |                | `*`            | `PRT`      | `PRT`       |
| wait_until   |              |               |                | `*`            | `P`        | `P`         |
| id           |              |               |                |                | `RT`       | `RT`        |
| duration     |              |               |                |                | `T`        | `T`         |
| size         |              |               |                |                | `T`        | `T`         |

Where:

* `*` means that the template parameter is available for the filter
* `S` means that the template parameter is available for the filter on service streaming
* `P` means that the template parameter is available for the filter on program streaming
* `R` means that the template parameter is available for the filter on streaming from a record
* `T` means that the template parameter is available for the filter on streaming from a timeshift
  record
* `L` means that the template parameter is available for the filter on streaming from a timeshift
  recorder
* Empty cell means that the template parameter is **NOT** available for the filter

### filters.tuner-filter

A filter which can be used for processing TS packets from a tuner command before
broadcasting the TS packets to subscribers.

This filter will be used not only for streaming API endpoints but also
background jobs if it's defined.

For example, this filter can be used for the drop-check for each tuner.

### filters.service-filter

A filter to drop TS packets which are not included in a specified service.

This filter will be used in the following streaming API endpoints:

* [/api/channels/{channel_type}/{channel}/services/{sid}/stream](./web-api.md#apichannelschannel_typechannelservicessidstream)
* [/api/services/{id}/stream](./web-api.md#apiservicesidstream)
* [/api/programs/{id}/stream](./web-api.md#apiprogramsidstream)

### filters.decode-filter

A filter to decode TS packets.

The `decode` query parameter for each streaming API endpoint configures the
decode-filter of the streaming.

### filters.program-filter

A filter to control streaming for a specified program.

This filter starts streaming when the program starts and stops streaming when
the program ends.

This filter will be used in the following streaming API endpoints:

* [/api/programs/{id}/stream](./web-api.md#apiprogramsidstream)

### pre-filters

A map of named filters which can be inserted at the input-side endpoint of the
filter pipeline.

The `pre-filters` query parameter for each streaming API endpoint configures the
pre-filters of the streaming.

The following request:

```
curl 'http://mirakc:40772/api/programs/{id}/stream?decode=1&pre-filters[0]=record'
```

will build the following filter pipeline:

```
pre-filters.record | decode-filter | program-filter
```

Pre-filters may change TS packets in the stream, but must not add and remove ones in the stream.

### post-filters

A map of named filters which can be inserted at the output-side endpoint of the
filter pipeline.

The `post-filters` query parameter for each streaming API endpoint configures
the post-filters of the streaming.

The following request:

```
curl 'http://mirakc:40772/api/programs/{id}/stream?decode=1&post-filters[0]=transcode'
```

will build the following filter pipeline:

```
decode-filter | program-filter | post-filters.transcode
```

Post-filters may change, add and remove TS packets in the stream, and also may change the
content-type of the stream.

## jobs

Definitions for background jobs.

Each job definition has the following properties:

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

## recording

### recording.basedir

`recording.basedir` specifies an absolute path to the base directory which is:

* Used for storing information about recording schedules
* Used as a base path in order to resolve relative paths in the
  `options.contentPath` property used in the Web endpoints for recording

Additionally, specifying an absolute path to an existing directory enables web
endpoints for recording functions.

The following files are stored in the base directory specified by this property:

* `schedules.json` contains recording schedules

You can specify multiple nested directories in the `options.contentPath`
property in a JSON data used in the following Web endpoints:

* [POST /api/recording/schedules](./web-api.md#postapirecordingschedules)

You can use this feature in order to output recorded content files to a
particular folder.

For example, the following JSON data sent to the Web endpoints above

```json
{
  "programId": 327360102415397,
  "options": {
    "contentPath": "videos/path/to/filename.m2ts"
  }
}
```

will record the content of the TV program under in the
`<recording.basedir>/videos` folder.  If you mount a shared folder on a NAS
server onto `<recording.basedir>/videos`, the content will be saved on the NAS.

### recording.records-dir

`recording.records-dir` is used together with `recording.basedir` and specifies an absolute path to
a holder where records for recorded TV programs are stored.

```
<recording.records-dir>
  +-- <record.id>.record.json
```

You can specify a holder inside `recording.basedir` to `recording.records-dir` if you want.

The following web endpoinds are enabled when `recording.basedir` and `recording.records-dir` are
specified:

* `GET /api/recording/records`
* `GET /api/recording/records/{id}`
* `DELETE /api/recording/records/{id}`
* `GET /api/recording/records/{id}/stream`
* `HEAD /api/recording/records/{id}/stream`

## timeshift

The timeshift recording of mirakc is a similar function to the Timeshift Machine
implemented on TVs and recorders produced by Toshiba.  The timeshift recording
records TS packets in a service stream into a fixed-size file used as a 'ring'
buffer.  User can playback TV programs recorded in the file until they are
purged due to the file size limit.

The timeshift recording has the following limitations:

* Duration of a record currently being recorded is updated only when a chunk is
  filled or the TV program ends

```yaml
# YAML
timeshift:
  recorders:
    nhk-bs:
      service-id: 400101  # NHK BS
      ts-file: /path/to/nhk-bs.timeshift.m2ts
      data-file: /path/to/nhk-bs.timeshift.data
      num-chunks: 4000  # about 616GB
      uses:
        tuner: tuner-for-nhk-bs-timeshift-recording
        channel-type: BS
        channel: BS15_0
```

```toml
# TOML
[timeshift.recorders.nhk-bs]
service-id = 400101  # NHK BS
ts-file = "/path/to/nhk-bs.timeshift.m2ts"
data-file = "/path/to/nhk-bs.timeshift.data"
num-chunks = 4000  # about 616GB

[timeshift.recorders.nhk-bs.uses]
tuner = "tuner-for-nhk-bs-timeshift-recording"
channel-type = "BS"
channel = "BS15_0"
```

### timeshift.recorders

Definitions of timeshift recorders.

* service-id
  * A service ID of a service stream to record
* ts-file
  * An absolute path to a file used as a ring buffer to record TS packets
* data-file
  * An absolute path to a file to save data like records encoded with JSON
* chunk-size
  * Size of a data chunk
  * Must be a multiple of `8192`
  * Recommended to specify a number of a multiple of `8192 * 188`
* num-chunks
  * The number of chunks in the ts-file
    * The maximum size of the ts-file is computed by `chunk-size * num-chunks`
  * Must be larger than 2
* num-reserves
  * The number of chunks in the gap between the head and the tail of the ring buffer
  * Must be larger than 0
  * `num-chunks - num-reserves` must be larger than 1
* priority
  * The priority of streaming
  * Should be larger than 0
* uses.tuner, uses.channel-type, uses.channel
  * A tuner name to use
  * The specified tuner will be dedicated for streaming for the specified channel

The following values are stored in the `data-file`:

* service-id
* chunk-size
* `num-chunks - num-reserves` (The maximum number of available chunks in the ts-file)

If the values above don't match ones in the recorder's configuration, mirakc
output error messages and terminates before the recorder starts.  In this case,
you have to select one of the following solutions, and then relaunch mirakc with
new configuration values:

1. Remove the data-file
2. Recover the data-file by using `mirakc rebuild-timeshift` and the ts-file

See the command help shown by `mirakc rebuild-timeshift --help` for the details
of this command.

## onair-program-trackers

Definitions of on-air TV program trackers which can be used for tracking the
current and next TV programs of a particular service.

At this point, the following trackers are available:

* Local trackers

### Local Tracker

A local tracker tracks on-air TV programs by collecting EIT [p/f] sections of a
service every minute using a local tuner.

```yaml
# YAML
tuners:
  # The `gr-tracker` tuner is used only by the `gr` local tracker.
  - name: gr-tracker
    types: [GR]
    command: >-
      recpt1 --device /dev/px4video2 {{{channel}}} {{{duration}}} -

onair-program-trackers:
  # The `gr` local tracker tracks on-air programs of every services
  # found by `jobs.scan-services` executed on `GR` channels.
  gr:
    local:
      channel-types: [GR]
      uses:
        tuner: gr-tracker
```

```toml
# TOML
[[tuners]]
# The `gr-tracker` tuner is used only by the `gr` local tracker.
name = "gr-tracker"
types = [ "GR" ]
command = "recpt1 --device /dev/px4video2 {{{channel}}} {{{duration}}} -"

# The `gr` local tracker tracks on-air programs of every services
# found by `jobs.scan-services` executed on `GR` channels.
[onair-program-trackers.gr.local]
channel-types = [ "GR" ]
uses = { tuner = "gr-tracker" }
```

The following properties can be specified:

* channel-types (required)
  * A list of channel types that the local tracker handles
* services (default: an empty list)
  * A list of service IDs that the local tracker handles
* excluded-services (default: an empty list)
  * A list of service IDs that then local tracker doesn't handles
* command (default: `timeout 5s mirakc-arib collect-eitpf --sids={{{sid}}}`)
  * A Mustache template string of a command which collects EIT[p/f] sections in
    NDJSON from a TS stream
  * See the description of `mirakc-arib collect-eitpf -h` for details of the
    JSON format
* uses.tuner (required)
  * A tuner name to use
  * The specified tuner will be dedicated for the local tracker and never shared with others

Template variables for `command`:

* sid
  * The 16-bit integer identifier of a service (SID)

As described above, a local tracker checks on-air TV programs **every minute**.
So, the `command` **SHOULD** be done within 1 minute.  If an execution of the
`command` takes more than 1 minute, the next periodic execution will be skipped.
You can use the `services` and `excluded-services` properties to limit services
handling by a tracker in order to reduce the execution time of the `command`.

Note that many logs will be output if you use a local tracker because it
executes the `command` every minute.

### Remote Tracker

A remote tracker tracks on-air TV programs by listening events from the `events`
Web endpoint.

```yaml
# YAML
onair-program-trackers:
  remote:
    remote:
      url: http://remote:40772/
```

```toml
# TOML
[onair-program-trackers.remote.remote]
url = "http://remote:40772/"
```

The following properties can be specified:

* url (required)
  * An URL of a remote server which provides web endpoints compatible with
    `/events` and `/api/onair` (and `/api/onair/{serviceId}`)
* services (default: an empty list)
  * A list of service IDs that the local tracker handles
* excluded-services (default: an empty list)
  * A list of service IDs that then local tracker doesn't handles
* events-endpoint (default: `/events`)
  * A web endpoint compatible with `/events`
* onair-endpoint (default: `/api/onair`)
  * A web endpoint compatible with `/api/onair` (and `/api/onair/{serviceId}`)

`MirakurunProgram` is not compatible with `EpgProgram`.  So, some of the
information might be lost.

## resource

### resource.strings-yaml

`resource.strings-yaml` specifies a path to a YAML file which contains strings
used in mirakc at runtime.

> TODO: This might be obsoleted by other tools like GNU gettext in the future.

### resource.logos

`resource.logos` specifies a logo image for each service.

```yaml
# YAML
resource:
  logos:
    - service-id: 400101
      image: /path/to/nhk-bs.png  # you can use any format of image
```

```toml
# TOML
[[resource.logos]]
service-id = 400101
image = "/path/to/nhk-bs.png"  # you can use any format of image
```

mirakc does not collect logo images from TS streams at runtime.  However, you
can find tools for that.  Or you can download logo images from somewhere.

Specified logo images are provided from the `/api/services/{id}/logo` endpoint.
Endpoint URLs are specified in a M3U8 playlist provided from the
`/api/iptv/playlist` endpoint and a XMLTV document provided from the
`/api/iptv/epg` endpoint.
