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

The following example launches the `samba` container to share timeshift record files which are
recorded in the `mirakc` container are exposed to the filesystem on the Docker host from the
`mirakc-timeshift-fs` container:

```yaml
version: '3.7'

x-environment: &default-environment
  TZ: Asia/Tokyo

x-logging: &default-logging
  driver: json-file
  options:
    max-size: '10m'
    max-file: '5'

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
        source: /path/to/timeshift-fs
        target: /mnt
        bind:
          propagation: rshared
    environment:
      <<: *default-environment
      RUST_LOG: info
    logging:
      <<: *default-logging

  samba:
    depends_on:
      - mirakc-timeshift-fs
    container_name: samba
    image: dperson/samba
    command:
      # Allow access only from LAN (192.168.0.0/16)
      - '-g'
      - 'hosts allow = 192.168.'
      # Expose as the timeshift folder
      - '-s'
      -'timeshift;/mnt'
    init: true
    restart: unless-stopped
    ports:
      - '139:139'
      - '445:445'
    volumes:
      - /path/to/timeshift-fs:/mnt:ro
    environment:
      <<: *default-environment
    logging:
      <<: *default-logging
```

The `samba` container must start after the `mirakc-timeshift-fs` container mounts the timeshift
filesystem onto `/path/to/mirakc/timeshift-fs` on the Docker host filesystem.
