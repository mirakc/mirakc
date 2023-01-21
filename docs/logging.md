# Logging

Log messages are output to `stdout` and `config.yml` has no properties to change
the output destination of log messages.

mirakc uses [log] and [tracing] for logging.  So, you can change the log level
of each module via the `RUST_LOG` environment variable like below:

```shell
export RUST_LOG=info,mirakc=debug
```

There are two logging formats as described in subsections below.

## `text` logging format

The `text` logging format is a single-line human-readable text format like
below:

```console
2020-04-15T14:16:26.257932+09:00   INFO mirakc::tuner: Loading tuners...
```

The `text` logging format is the default logging format.

## `json` logging format

The `json` logging format is a single-line JSON format having the following
properties:

```jsonc
{
  // High-Resolution timestamp represented by "<unix-time-secs>.<nanos>"
  "timestamp": "1586928063.723044864",
  "level": "INFO",
  "target": "mirakc::tuner",
  "fields": {
    "message": "Loading tuners...",
    "log.target": "mirakc::tuner",
    "log.module_path": "mirakc::tuner",
    "log.file": "src/tuner.rs",
    "log.line": 74
  }
}
```

The `json` logging format is enabled with the `--log-format=json` command-line
option or the `MIRAKC_LOG_FORMAT=json` environment variable.

The `json` logging format is intended to be used for a log analysis.  For
example:

* It can be used as an input for a distribute tracing system like Zipkin
* It can be used as data source for collecting statistics provided to a
  monitoring system like Prometheus

[log]: https://crates.io/crates/log
[tracing]: https://github.com/tokio-rs/tracing

## Logging from child processes

Logging from child processes is disabled by default.  Defining the following
environment variables enables it:

```shell
export RUST_LOG='info,[pipeline]=debug'
export MIRAKC_DEBUG_CHILD_PROCESS=1
export MIRAKC_ARIB_LOG=warn,filter-program=info
export MIRAKC_ARIB_LOG_NO_TIMESTAMP=1
```

See [mirakc/mirakc-arib](https://github.com/mirakc/mirakc-arib#logging)
about log levels which can be used in the `MIRAKC_ARIB_LOG` environment
variable.
