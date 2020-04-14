# Logging

Log messages are output to `stdout` and `config.yml` has no properties to change
the output destination of log messages.

mirakc uses [log] and [tracing] for logging.  So, you can change the log level
of each module via the `RUST_LOG` environment variable like below:

```shell
export RUST_LOG=info,mirakc=debug
```

There are several environment variables to change the logging format:

* `MIRAKC_LOG_FORMAT`
  * `json` or `text` (default)
* `MIRAKC_LOG_TIMESTAMP`
  * `hrtime` or `localtime` (default)

[log]: https://crates.io/crates/log
[tracing]: https://github.com/tokio-rs/tracing
[issues/6]: https://github.com/masnagam/mirakc/issues/6

## Logging from child processes

Logging from child processes is disabled by default.  Defining the following
environment variables enables it:

```shell
export MIRAKC_DEBUG_CHILD_PROCESS=
export MIRAKC_ARIB_LOG=info
```

See [masnagam/mirakc-arib](https://github.com/masnagam/mirakc-arib#logging)
about log levels which can be used in the `MIRAKC_ARIB_LOG` environment
variable.
