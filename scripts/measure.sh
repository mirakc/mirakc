#!/bin/sh

PROGNAME=$(basename $0)
BASEDIR=$(cd $(dirname $0); pwd)

TARGET="$1"
DURATION="$2"

CPU_EXPR='100 * (1 - avg(irate(node_cpu_seconds_total{mode="idle"}[1m])))'
MEMORY_EXPR='node_memory_MemTotal_bytes - (node_memory_MemFree_bytes + node_memory_Buffers_bytes + node_memory_Cached_bytes)'
LOAD1_EXPR='node_load1'
TX_EXPR='irate(node_network_transmit_bytes_total{device=~"eth0|enp6s0"}[1m]) * 8'
RX_EXPR='irate(node_network_receive_bytes_total{device=~"eth0|enp6s0"}[1m]) * 8'

stream() {
  node $BASEDIR/perf-metrics stream "$TARGET" "$DURATION"
}

system() {
  node $BASEDIR/perf-metrics system "$1" "$2"
}

cpu() {
  system cpu "$CPU_EXPR"
}

memory() {
  system memory "$MEMORY_EXPR"
}

load1() {
  system load1 "$LOAD1_EXPR"
}

tx() {
  system tx "$TX_EXPR"
}

rx() {
  system rx "$RX_EXPR"
}

summary() {
  node $BASEDIR/perf-metrics summary
}

graph_url() {
  node $BASEDIR/perf-metrics prom-graph-url
}

stream | cpu | memory | load1 | tx | rx | summary | graph_url
