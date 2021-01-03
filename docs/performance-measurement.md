# Performance Measurement

The performance metrics shown in [README.md](../README.md) were collected by
using the following command executed on a local PC:

```console
$ ./scripts/measure http://mirakc:40772 >/dev/null
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
mx       754524464   4013428   0
cx       1027186880  5463760   0
ex       1061083280  5644060   0
tx       1022076476  5436577   0
bs-ntv   1013682088  5391926   0
bs-ex    1094240276  5820427   0
bs-tbs   1074745616  5716732   0
bs11     1395304792  7421834   0

NAME    MIN                 MAX
------  ------------------  ------------------
cpu     31.790845305783343  36.841051648131206
memory  232095744           233185280
load1   1.36                2.84
tx      111210158.4         121260930.33797747
rx      1020067.2           1085208.5333333334

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
  * Default `server.workers` (4 = the number of CPU cores)
  * `MALLOC_ARENA_MAX=2`
* Tuner
  * PLEX PX-Q3U4
* Docker
  * version 19.03.1, build 74b1e89
  * Server processes were executed in a docker container

where a Prometheus server was running on the local PC.

[scripts/measure](../scripts/measure) performs:

* Receiving TS streams from 4 GR and 4 BS services for 10 minutes
  * `cat` is used as post-filter
* Collecting system metrics by using [Prometheus] and [node_exporter]
* Counting the number of dropped TS packets by using [node-aribts]

The script should be run when the target server is idle.

You can spawn a Prometheus server using a `docker-compose.yml` in the
[docker/prometheus](../docker/prometheus) folder if you have never used it
before.  See [this document](../docker/prometheus/README.md) in the folder
before using it.

Several hundreds or thousands of dropped packets were sometimes detected during
the performance measurement.  The same situation occurred in Mirakurun.

[Armbian]: https://www.armbian.com/rock64/
[px4_drv]: https://github.com/nns779/px4_drv
[Prometheus]: https://prometheus.io/
[node_exporter]: https://github.com/prometheus/node_exporter
[node-aribts]: https://www.npmjs.com/package/aribts
