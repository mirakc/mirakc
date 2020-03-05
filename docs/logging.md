# Logging

Log messages are output to `stderr` and `config.yml` has no properties to change
the output destination of log messages.

mirakc uses [log] and [env_logger] for logging.  So, you can change the log
level of each module via the `RUST_LOG` environment variable like below:

```shell
export RUST_LOG=info,mirakc=debug
```

See documents of [env_logger] for details.

The format of log messages may change in the future.  See [issues/6].

[log]: https://crates.io/crates/log
[env_logger]: https://crates.io/crates/env_logger
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
