# docker-prometheus

## How to use

Create `.env` file in this folder, which defines the following environment
variables:

```console
$cat .env
TARGET_IP=192.168.1.23
TZ=Asia/Tokyo
```

The `TARGET_IP` environment variables defines an IP address of a target server
for a performance measurement using
[scripts/measure.sh](../../scripts/measure.sh).

Create a `mirakc-prometheus` container with the `mirakc_prometheus_network`
network and the `mirakc_prometheus_data` volume, and start them in the
background:

```console
$ docker-compose up -d
```

Show logs:

```console
$ docker-compose logs -f --tail=10
```

Stop the container, and remove it together with the network and the volume:

```console
$ docker-compose down -v
```
