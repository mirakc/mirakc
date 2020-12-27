# How to debug

It's recommended to use [VS Code] for debugging.

There are two folders which contains settings regarding VS Code:

* [.devcontainer](./.devcontainer) contains settings for
  [VS Code Remote Containers]
* [.vscode](./.vscode) contains basic settings

Currently, the following debugging environments are supported:

* Debugging on the local machine
* Debugging with a remote container

## Debugging on the local machine

Export environment variables as described in `.vscode/settings.json`:

```shell
export MIRAKC_DEV_RUSTC_COMMIT_HASH="$( \
  rustc -vV | grep 'commit-hash' | cut -d ' ' -f2)"
export MIRAKC_DEV_RUST_TOOLCHAIN_PATH="$( \
  rustup toolchain list -v | head -1 | cut -f2)"
```

## Debugging with a remote container

Before starting to debug using VS Code Remote Containers, you need to create
`mirakurun.openapi.json` in the project root folder with the following command:

```shell
./scripts/mirakurun-openapi-json -c >mirakurun.openapi.json
```

and then create and edit `.devcontainer/config.yml`:

```shell
vi .devcontainer/config.yml
```

## Launch configurations

The following 3 configurations have been defined in `.vscode/launch.json`:

* Debug
* Debug w/ child processes (Debug + log messages from child processes)
* Debug unit tests

`SIGPIPE` never stops the debugger.  See `./vscode/settings.json`.

[VS Code]: https://code.visualstudio.com/
[VS Code Remote Containers]: https://code.visualstudio.com/docs/remote/containers
