# Web API

Web API endpoints listed below have been implemented at this moment:

| ENDPOINT                                        | COMPATIBLE WITH MIRAKURUN? |
|-------------------------------------------------|----------------------------|
| [GET /api/version]                              | :heavy_check_mark:         |
| [GET /api/status]                               |                            |
| [GET /api/channels]                             | :heavy_check_mark:         |
| [GET /api/channels/{channel_type}/{channel}/stream]| :heavy_check_mark:      |
| [GET /api/channels/{channel_type}/{channel}/services/{sid}/stream]|          |
| [GET /api/services]                             | :heavy_check_mark:         |
| [GET /api/services/{id}]                        | :heavy_check_mark:         |
| [GET /api/services/{id}/logo]                   | :heavy_check_mark:         |
| [GET /api/services/{id}/programs]               |                            |
| [GET /api/services/{id}/stream]                 | :heavy_check_mark:         |
| [GET /api/programs]                             | :heavy_check_mark:         |
| [GET /api/programs/{id}]                        | :heavy_check_mark:         |
| [GET /api/programs/{id}/stream]                 | :heavy_check_mark:         |
| [GET /api/tuners]                               | :heavy_check_mark:         |
| [GET /api/docs]                                 | :heavy_check_mark:         |
| [GET /api/iptv/playlist]                        | :heavy_check_mark:         |
| [GET /api/iptv/channel.m3u8]                    |                            |
| [GET /api/iptv/epg]                             |                            |
| [GET /api/iptv/xmltv]                           | :heavy_check_mark:         |
| [GET /api/recording/schedules]                  |                            |
| [POST /api/recording/schedules]                 |                            |
| [GET /api/recording/schedules/{program_id}]     |                            |
| [DELETE /api/recording/schedules/{program_id}]  |                            |
| [GET /api/recording/recorders]                  |                            |
| [POST /api/recording/recorders]                 |                            |
| [GET /api/recording/recorders/{program_id}]     |                            |
| [DELETE /api/recording/recorders/{program_id}]  |                            |
| [GET /api/timeshift]                            |                            |
| [GET /api/timeshift/{recorder}]                 |                            |
| [GET /api/timeshift/{recorder}/records]         |                            |
| [GET /api/timeshift/{recorder}/records/{record}]|                            |
| [GET /api/timeshift/{recorder}/stream]          |                            |
| [GET /api/timeshift/{recorder}/records/{record}/stream]|                     |

The endpoints above are enough to run [EPGStation].

It also enough to run [BonDriver_mirakc].  It's strongly recommended to
enable `SERVICE_SPLIT` in `BonDriver_mirakc.ini` in order to reduce network
traffic between mirakc and BonDriver_mirakc.  Because the
`/api/channels/{channel_type}/{channel}/stream` endpoint provides a TS stream
which includes all services in the specified channel.

Unfortunately, mirakc doesn't work with [BonDriver_Mirakurun] at this point due
to some kind of issue in BonDriver_Mirakurun or mirakc.
See [issues/4](https://github.com/mirakc/mirakc/issues/4) for details
(discussion in Japanese).

Web API endpoints listed below have been implemented as the mirakc extensions:

* [GET /api/services/{id}/programs]
* [GET /api/iptv/playlist]
* [GET /api/recording/schedules]
* [POST /api/recording/schedules]
* [GET /api/recording/schedules/{program_id}]
* [DELETE /api/recording/schedules/{program_id}]
* [GET /api/recording/recorders]
* [POST /api/recording/recorders]
* [GET /api/recording/recorders/{program_id}]
* [DELETE /api/recording/recorders/{program_id}]
* [GET /api/timeshift]
* [GET /api/timeshift/{recorder}]
* [GET /api/timeshift/{recorder}/records]
* [GET /api/timeshift/{recorder}/records/{record}]
* [GET /api/timeshift/{recorder}/stream]
* [GET /api/timeshift/{recorder}/records/{record}/stream]

[GET /api/version]: #getapiversion
[GET /api/status]: #getapistatus
[GET /api/channels]: #getapichannels
[GET /api/channels/{channel_type}/{channel}/stream]: #getapichannelschannel_typechannelstream
[GET /api/channels/{channel_type}/{channel}/services/{sid}/stream]: #getapichannelschannel_typechannelservicessidstream
[GET /api/services]: #getapiservices
[GET /api/services/{id}]: #getapiservicesid
[GET /api/services/{id}/logo]: #getapiservicesidlogo
[GET /api/services/{id}/programs]: #getapiservicesidprograms
[GET /api/services/{id}/stream]: #getapiservicesidstream
[GET /api/programs]: #getapiprograms
[GET /api/programs/{id}]: #getapiprogramsid
[GET /api/programs/{id}/stream]: #getapiprogramsidstream
[GET /api/tuners]: #getapituners
[GET /api/docs]: #getapidocs
[GET /api/iptv/playlist]: #getapiiptvplaylist
[GET /api/iptv/channel.m3u8]: #getapiiptvchannelm3u8
[GET /api/iptv/epg]: #getapiiptvepg
[GET /api/iptv/xmltv]: #getapiiptvxmltv
[GET /api/recording/schedules]: #getapirecordingschedules
[POST /api/recording/schedules]: #postapirecordingschedules
[GET /api/recording/schedules/{program_id}]: #getapirecordingschedulesprogram_id
[DELETE /api/recording/schedules/{program_id}]: #deleteapirecordingschedulesprogram_id
[GET /api/recording/recorders]: #getapirecordingrecorders
[POST /api/recording/recorders]: #postapirecordingrecorders
[GET /api/recording/recorders/{program_id}]: #getapirecordingrecordersprogram_id
[DELETE /api/recording/recorders/{program_id}]: #deleteapirecordingrecordersprogram_id
[GET /api/timeshift]: #getapitimeshift
[GET /api/timeshift/{recorder}]: #getapitimeshiftrecorder
[GET /api/timeshift/{recorder}/records]: #getapitimeshiftrecorderrecords
[GET /api/timeshift/{recorder}/records/{record}]: #getapitimeshiftrecorderrecordsrecord
[GET /api/timeshift/{recorder}/stream]: #getapitimeshiftrecorderstream
[GET /api/timeshift/{recorder}/records/{record}/stream]: #getapitimeshiftrecorderrecordsrecordstream

## Incompatibility of the `X-Mirakurun-Priority` header

There are the following differences of the `X-Mirakurun-Priority` header between
mirakc and Mirakurun:

* Treated as 0 when the minimum value of `X-Mirakurun-Priority` headers is less
  than 0
* Treaded as 128 when the maximum value of `X-Mirakurun-Priority` headers is
  greater than 0
* Can grab a tuner which is used by other users regardless of their priorities
  if the priority is 128

## Incompatibility of the `decode` query parameter

Before `1.0.30`, mirakc does **NOT** decode the stream when no `decode` query
parameter is specified.  This behavior is **incompatible** with Mirakurun.

This incompatibility was fixed in `1.0.30`.

## GET /api/version

Returns the **current** version in the same JSON format as Mirakurun.

The `latest` property is not supported and shows the current version.

## GET /api/status

Returns an empty object.

## GET /api/channels

Returns a list of channels.

Query parameters have **NOT** been supported.

## GET /api/channels/{channel_type}/{channel}/stream

Starts streaming for a channel.

## GET /api/channels/{channel_type}/{channel}/services/{sid}/stream

Starts streaming for a service in a channel.

Unlike Mirakurun, the `sid` must be a service ID.  In Mirakurun, the `sid` is a
service ID or the ID of a `ServiceItem` class.

## GET /api/services

Returns a list of services.

Query parameters have **NOT** been supported.

## GET /api/services/{id}

Returns a service.

## GET /api/services/{id}/logo

Returns a logo image if available.

Support GET and HEAD methods so that IPTV Simple Client in Kodi works properly.

## GET /api/services/{id}/programs

Returns a list of programs of a particular service.

## GET /api/services/{id}/stream

Starts streaming for a service.

Support GET and HEAD methods so that IPTV Simple Client in Kodi works well.

## GET /api/programs

Returns a list of programs.

Query parameters have **NOT** been supported.

## GET /api/programs/{id}

Returns a program.

## GET /api/programs/{id}/stream

Starts streaming for a program.

The streaming will start when the program starts and stops when the program
ends.

## GET /api/tuners

Returns a list of tuners.

Query parameters have **NOT** been supported.

## GET /api/docs

Returns an OpenAPI JSON data that is compatible with one generated by Mirakurun.

The compatibility is very important for working with applications which use
`mirakurun.Client`.  See also [issues/13](https://github.com/mirakc/mirakc/issues/13).

## Web API endpoints for IPTV clients

Using these endpoints, you can integrate mirakc with IPTV clients which support
the M3U8 playlist and the XMLTV document.

For example, you can integrate mirakc with PVR IPTV Simple Client in Jodi with
the following settings:

* General | M3U Play List URL
  * `http://<host>:<port>/api/iptv/playlist`
* EPG Settings | XMLTV URL
  * `http://<host>:<port>/api/iptv/epg`

After rebooting the Kodi, the following features will be available:

* TV
  * Channels
  * Guide
* Radio (if channels defined in `config.yml` have audio-only services)
  * Channels
  * Guide

### GET /api/iptv/playlist

Returns a M3U8 playlist which includes all TV services.

The format of the M3U8 playlist is compatible with EPGStation.

The following query parameters can be specified:

* pre-filters
* post-filters

The specified query parameters are added to URLs in the playlist.

### GET /api/iptv/channel.m3u8

Alias of [GET /api/iptv/playlist].  Added for compatibility with EPGStation.

### GET /api/iptv/epg

Returns a XMLTV document which contains TV program information for a specified
number of days.

The following query parameters can be specified:

* days (1-8, default: 3)

[EPGStation]: https://github.com/l3tnun/EPGStation
[BonDriver_mirakc]: https://github.com/epgdatacapbon/BonDriver_mirakc

### GET /api/iptv/xmltv

Alias of [GET /api/iptv/epg].  Added for compatibility with Mirakurun.

Unlike `/api/iptv/epg`, this endpoint does not support the `days` query
parameter for compatibility with Mirakurun and returns all programs.

## Web API endpoints for recording

mirakc doesn't provide the following endpoints:

* Listing recorded TV programs
* Getting metadata of a recorded TV program
* Streaming a recorded TV program

These functions are out of scope of mirakc.  Because there already exist some
media center applications that provide better functions to manage media files
than mirakc.

If you don't like to use any media center applications, you can simply mount
a folder specified in `config.recording.records-dir` (or
`config.recording.contents-dir`) onto somewhere like below:

```yaml
server:
  mounts:
    /videos:
      # Recorded media files will be served as static files.
      path: /path/to/videos
      listing: true

recording:
  records-dir: /var/lib/mirakc/recording
  # Recorded media files will be stored in /path/to/videos.
  contents-dir: /path/to/videos
```

### GET /api/recording/schedules

Returns a list of recording schedules.

### POST /api/recording/schedules

Creates a recording schedule.

### GET /api/recording/schedules/{program_id}

Returns a recording schedule for a specified program.

### DELETE /api/recording/schedules/{program_id}

Deletes a recording schedule for a specified program.

### GET /api/recording/recorders

Returns a list of recorders.

### POST /api/recording/recorders

Start recording for a specified program.

### GET /api/recording/recorders/{program_id}

Returns a recorder for a specified program.

### DELETE /api/recording/recorders/{program_id}

Stop recording for a specified program.

## Web API endpoints for timeshift recording and playback

### GET /api/timeshift

Returns a list of timeshift recorders.

### GET /api/timeshift/{recorder}

Returns a timeshift recorder.

### GET /api/timeshift/{recorder}/records

Returns a list of records in a timeshift recorder.

### GET /api/timeshift/{recorder}/records/{record}

Returns a records in a timeshift recorder.

### GET /api/timeshift/{recorder}/stream

Starts live streaming for a timeshift recorder.

The following command starts live streaming from a specific record:

```
curl -sG http://mirakc:40772/api/timeshift/etv/stream?record=1
```

You can specify pre-filters and post-filters like any other endpoint for streaming.

### GET /api/timeshift/{recorder}/records/{record}/stream

Starts on-demand streaming for a record in a timeshift recorder.

The following command starts on-demand streaming from a specific record:

```
curl -sG http://mirakc:40772/api/timeshift/etv/records/1/stream
```

You can specify pre-filters and post-filters like any other endpoint for streaming.
You cannot seek the stream when you specify post-filters.
