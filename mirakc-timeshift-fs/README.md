# mirakc-timesift-fs

`mirakc-timeshift-fs` is a supplemental program which exports timeshift records as files on the
local filesystem using [FUSE].

The following command mounts the timeshift filesystem onto `/path/to/dir`:

```shell
mirakc-timeshift-fs -c /path/to/config.yml /path/to/dir
```

The directory structure is like below:

```
<mount-point>/
  |
  +-- <sanitized recorder.name>/
  |     |
  .     +-- "<record.id>.<sanitized record.program.name>.m2ts"
  .     |
  .     .
```

[FUSE]: https://en.wikipedia.org/wiki/Filesystem_in_Userspace

## Using Docker

Mount /dev/fuse and folders which contain files specified in
`config.timeshift.recorders[].ts-file` and `config.timeshift.recorders[].data-file`.

The following example launches the `dlna` container to share timeshift record files which are
recorded in the `mirakc` container are exposed to the filesystem on the Docker host from the
`mirakc-timeshift-fs` container:

```yaml
version: '3'

x-environment: &default-environment
  TZ: Asia/Tokyo

services:
  mirakc:
    ...
    volumes:
      - /path/to/config.yml:/etc/mirakc/config.yml:ro
      - /path/to/timeshift:/var/lib/mirakc/timeshift
    ...

  mirakc-timeshift-fs:
    container_name: mirakc-timeshift-fs
    image: mirakc/timeshift-fs
    init: true
    restart: unless-stopped
    cap_add:
      - SYS_ADMIN
    devices:
      - /dev/fuse
    volumes:
      # Use the same config.yml
      - /path/to/config.yml:/etc/mirakc/config.yml:ro
      # Timeshift files
      - /path/to/timeshift:/var/lib/mirakc/timeshift
      # Mount point
      - type: bind
        source: /media/timeshift
        target: /mnt
        bind:
          propagation: rshared
    environment:
      <<: *default-environment
      RUST_LOG: info

  dlna:
    container_name: dlna
    depends_on:
      - mirakc-timeshift-fs
    image: mirakc/minidlna
    restart: unless-stopped
    network_mode: host
    volumes:
      - /media/timeshift:/mnt:ro
      - minidlna-cache:/var/cache/minidlna
    environment:
      <<: *default-environment
```

The folders containing data files must be mounted with read-write so that the
`mirakc-timeshift-fs` can create a lock file for each data file in the same folder.

The `dlna` container must start after the `mirakc-timeshift-fs` container mounts the timeshift
filesystem onto `/media/timeshift` on the Docker host filesystem.
