# Inside mirakc

## Broadcaster

The `Broadcaster` is one of key types in a mechanism to share a tuner device
with multiple subscribers.

The `Broadcaster` manages a MPEG-TS streaming from a tuner device and accepts a
subscription from a subscriber who likes to receive the MPEG-TS streaming.  When
a data chunk arrives, the `Broadcaster` delivers it to subscribers.

Data chunks are delivered through a `tokio::sync::mpsc::channel` between the
`Broadcaster` and each subscriber.  The `tokio::sync::mpsc::channel` is also
used as a buffer for data chunks.

```
tuner-device
  |
  V
Broadcaster
  |
  +--> tokio::sync::mpsc::channel --> subscriber-1
  |
  +--> tokio::sync::mpsc::channel --> subscriber-2
```

## Streaming Pipeline

mirakc has a pipeline to process MPEG-TS packets.

```
  tuner-command (external process)
    |
+---V------ Broadcaster -----------------+
| tokio::sync::mpsc::channel (as buffer) |
+---|------------------------------------+
    V
  MpegTsStream
    |
    +--(when using filters)
    |      |
    |  +---V-------- CommandPipeline --------------+
    |  | pre-filter (external process) [optional]  |
    |  |   |                                       |
    |  |   V                                       |
    |  | service/program-filter (external process) |
    |  |   |                                       |
    |  |   V                                       |
    |  | post-filter (external process) [optional] |
    |  +---|---------------------------------------+
    |      V
    |    tokio::sync::mpsc::channel (as buffer) <web-buffer>
    |      |
    +------+
    |
    V
  actix-web (HTTP chunk encoding)
    |
    V
  client
```

Writing data to the input-side endpoint of the `CommandPipeline` and reading
data from the output-side endpoint of the `CommandPipeline` are processed by
individual asynchronous tasks.

Data transfer between adjacent processes in the `CommandPipeline` is performed
through a UNIX pipe.  And data is copied synchronously.  This means that writing
to `stdout` in the upstream process is blocked when the buffer of the UNIX pipe
is full.

The `web-buffer` can be tuned by the following configuration properties:

* [server.stream-chunk-size](./config.md#server.stream-chunk-size)
* [server.stream-max-chunks](./config.md#server.stream-max-chunks)

See also the Japanese discussion on
[issues/18](https://github.com/masnagam/mirakc/issues/18).
