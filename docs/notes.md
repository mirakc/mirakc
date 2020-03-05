# Notes

## mirakc leaks memory?

The memory usage of mirakc may look increasing when it runs for a long time.  If
you see this, you may suspect that mirakc is leaking memory.  But, don't worry.
mirakc does **NOT** leak memory.  In fact, the increase in the memory usage will
stop at some point.

mirakc uses system memory allocator which is default in Rust.  In many cases,
`malloc` in `glibc` is used.  The recent `malloc` in `glibc` is optimized for
multithreading.  And it doesn't free unused memory blocks in some situations.
This is the root cause of the increase in memory usage of mirakc.

### Use an Alpine-base image

The simplest solution is using an Alpine-based image.  As you can see in
[the previous section](#after-running-for-1-day), the memory usage can be
drastically improved.

Performance of the memory allocation in the multi-threading environment may be
degraded.  However, there seems to be no significant performance degradation on
ROCK64.

### Tune the glibc allocator by using environment variables

There are environment variables to control criteria for freeing unused memory
blocks as described in [this page](http://man7.org/linux/man-pages/man3/mallopt.3.html).

For example, setting `MALLOC_TRIM_THRESHOLD_=-1` may improve the increase in
memory usage that occurs when accessing the `/api/programs` endpoint.

## Why use external commands to process TS packets?

Unfortunately, there is no **practical** MPEG-TS demuxer written in Rust at this
moment.

mirakc itself has no functionalities to process TS packets at all.  Therefore,
nothing can be done with mirakc alone.

For example, mirakc provides an API endpoint which returns a schedule of TV
programs, but mirakc has no functionality to collect EIT tables from TS streams.
mirakc just delegates that to an external program which is defined in the
`jobs.update-schedules.command` property in the configuration YAML file.

Of course, this design may make mirakc less efficient because using child
processes and pipes between them increases CPU and memory usages.  But it's
already been confirmed that mirakc is efficient enough than Mirakurun as you see
previously.

This design may be changed in the future if someone creates a MPEG-TS demuxer
which is functional enough for replacing the external commands.
