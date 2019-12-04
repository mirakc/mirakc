'use strict';

const aribts = require('aribts');
const http = require('http');
const fetch = require('node-fetch');
const ms = require('ms');
const querystring = require('querystring');
const streamToString = require('stream-to-string');
const Table = require('easy-table');

const program = require('commander');
const packageJson = require('./package.json');

const HELP = `
Description:
  Tools for performance measurements

Examples:
  $ mirakc-perf-metrics stream http://mirakc:40772 | \\
      mirakc-perf-metrics system cpu \\
        '100 * (1 - avg(irate(node_cpu_seconds_total{mode="idle"}[1m])))' | \\
      mirakc-perf-metrics system memory \\
        'node_memory_MemTotal_bytes - (node_memory_MemFree_bytes + node_memory_Buffers_bytes + node_memory_Cached_bytes)' | \\
      mirakc-perf-metrics system load1 \\
        'node_load1' | \\
      mirakc-perf-metrics system tx \\
        'irate(node_network_transmit_bytes_total{device=~"eth0|enp6s0"}[1m])' | \\
      mirakc-perf-metrics system rx \\
        'irate(node_network_receive_bytes_total{device=~"eth0|enp6s0"}[1m])'
`;

program
  .version(packageJson.version)
  .description(packageJson.description)
  .on('--help', () => console.log(HELP));

const STREAM_HELP = `
Arguments:
  target    Base URL of the target server
  duration  Duration for measurement like '10s' (default: '10m')

Description:
  ...
`;

program
  .command('stream <target> [duration]')
  .on('--help', () => console.log(STREAM_HELP))
  .action(collectMetrics);

const SYSTEM_HELP = `
Arguments:
  label  Label
  expr   Prometheus expression

Description:
  ...
`;

program
  .command('system <label> <expr>')
  .option(
    '--prom <url>',
    'URL of a Prometheus server collecting metrics from a node_exporter'
      + ' running on the target',
    'http://localhost:9090')
  .option(
    '--start <unix-timestamp-ms>',
    'Start timestamp')
  .option(
    '--end <unix-timestamp-ms>',
    'End timestamp')
  .option(
    '--step <duration-sec>',
    'Resolution step in seconds',
    parseInt, 10)
  .on('--help', () => console.log(SYSTEM_HELP))
  .action(collectSystemMetrics);

const SUMMARY_HELP = `
Description:
  ...
`;

program
  .command('summary')
  .on('--help', () => console.log(SUMMARY_HELP))
  .action(showSummary);

const PROM_GRAPH_URL_HELP = `
Description:
  ...
`;

program
  .command('prom-graph-url')
  .option(
    '--prom <prometheus>',
    'URL of a Prometheus server collecting metrics from a node_exporter'
      + ' running on the target',
    'http://localhost:9090')
  .on('--help', () => console.log(PROM_GRAPH_URL_HELP))
  .action(makePromGraphUrl);

program.parse(process.argv);

function timestamp() {
  const now = Date.now();
  return now - (now % 1000);
}

function collectMetrics(target, duration, options) {
  const CHANNELS = [
    { name: 'mx', sid: 3239123608 },
    { name: 'cx', sid: 3274001056 },
    { name: 'ex', sid: 3274101064 },
    { name: 'tx', sid: 3274201072 },
    { name: 'bs-ntv', sid: 400141 },
    { name: 'bs-ex', sid: 400151 },
    { name: 'bs-tbs', sid: 400161 },
    { name: 'bs11', sid: 400211 },
  ];

  if (!target) {
    console.error('target is required');
    process.exit(1);
  }

  var timeout = ms('10m');
  if (duration) {
    timeout = ms(duration);
  }

  // Other metrics can be collected via a Prometheus server.
  var metrics = {
    start: 0,
    end: 0,
    streams: {},
    system: [],
  };

  const start = timestamp();
  const timer = setTimeout(async () => await stop(), timeout);

  const reqs = CHANNELS.map((channel) => {
    console.error(`Reading TS packets from ${channel.name}...`);
    metrics.streams[channel.name] = {};
    const req = http.request(
      `${target}/api/services/${channel.sid}/stream?decode=1`);
    req.on('response', (res) => {
      const tsStream = new aribts.TsStream();
      tsStream.on('info', (info) => {
        collectStreamMetrics(info, metrics.streams[channel.name]);
      });
      var numBytes = 0;
      res.on('data', (data) => numBytes += data.length);
      res.on('end', () => metrics.streams[channel.name].numBytes = numBytes);
      res.pipe(tsStream);
      tsStream.resume();
    });
    req.end();
    return req;
  });

  function collectStreamMetrics(info, metrics) {
    metrics.numPackets = 0;
    metrics.numDrops = 0;
    for (const value of Object.values(info)) {
      metrics.numPackets += value.packet;
      metrics.numDrops += value.drop;
    }
  }

  async function stop() {
    metrics.start = start;
    metrics.end = timestamp();
    reqs.forEach((req) => req.abort());
    clearTimeout(timer);
  }

  process.on('SIGINT', async () => await stop());

  process.on('exit', () => {
    console.log(JSON.stringify(metrics));
  });
}

async function collectSystemMetrics(label, expr, options) {
  const metrics = JSON.parse(await streamToString(process.stdin));
  const params = querystring.stringify({
    query: expr,
    start: metrics.start / 1000,
    end: metrics.end / 1000,
    step: options.step,
  });
  const url = `${options.prom}/api/v1/query_range?${params}`;
  const res = await fetch(url);
  const json = await res.json();
  const values = json.data.result[0].values;
  metrics.system.push({ label, expr, step: options.step, values });
  console.log(JSON.stringify(metrics));
}

async function showSummary() {
  const metrics = JSON.parse(await streamToString(process.stdin));
  showStreamsMetrics(metrics);
  showSystemMetrics(metrics);
  console.log(JSON.stringify(metrics));

  function showStreamsMetrics(metrics) {
    const columns = [
      'channel', 'numBytes', 'numPackets', 'numDrops'
    ];
    const table = new Table();
    appendRow(table, columns, {
      channel: 'CHANNEL',
      numBytes: '#BYTES',
      numPackets: '#PACKETS',
      numDrops: '#DROPS',
    });
    table.pushDelimeter(columns);
    for (const channel in metrics.streams) {
      const met = metrics.streams[channel];
      appendRow(table, columns, {
        channel,
        numBytes: met.numBytes,
        numPackets: met.numPackets,
        numDrops: met.numDrops,
      });
    }
    console.error(table.print());
  }

  function showSystemMetrics(metrics) {
    const columns = [
      'name', 'min', 'max',
    ];
    const table = new Table();
    appendRow(table, columns, {
      name: 'NAME',
      min: 'MIN',
      max: 'MAX',
    });
    table.pushDelimeter(columns);
    const start = (metrics.start / 1000) + 120;
    const end = (metrics.end / 1000) - 120;
    for (const met of metrics.system) {
      const vals = met.values.filter((v) => v[0] >= start && v[0] <= end);
      appendRow(table, columns, {
        name: met.label,
        min: vals.reduce((a, v) => Math.min(a, Number(v[1])), Number.MAX_VALUE),
        max: vals.reduce((a, v) => Math.max(a, Number(v[1])), Number.MIN_VALUE),
      });
    }
    console.error(table.print());
  }

  function appendRow(table, columns, data, paddings = {}) {
    columns.forEach((col) => {
      let printer = undefined;
      if (paddings[col] === 'left') {
        printer = Table.padLeft;
      }
      if (paddings[col] === 'right') {
        printer = Table.padRight;
      }
      table.cell(col, data[col], printer);
    });
    table.newRow();
  }
}

async function makePromGraphUrl(options) {
  const metrics = JSON.parse(await streamToString(process.stdin));
  const range = ms(metrics.end - metrics.start + ms('2m'));
  const end = (new Date(metrics.end + ms('2m'))).toISOString();
  const params = {};
  for (var i = 0; i < metrics.system.length; i++) {
    const met = metrics.system[i];
    params[`g${i}.range_input`] = range;
    params[`g${i}.end_input`] = end;
    params[`g${i}.step_input`] = met.step;
    params[`g${i}.expr`] = met.expr;
    params[`g${i}.tab`] = 0;
  }
  console.error(`${options.prom}/graph?${querystring.stringify(params)}`);
  console.log(JSON.stringify(metrics));
}
