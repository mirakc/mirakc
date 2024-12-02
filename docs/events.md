# Events

mirakc provides the Web endpoint `/events` for event notifications using
Sever-Sent Events ([SSE]).

Using this feature, users can implement useful functions such as a rule-based
automatic recording scheduler like [this](https://github.com/mirakc/contrib/blob/main/recording/simple-rules.js).

## tuner.status-changed

An event sent when the status of a tuner is changed.

```jsonc
{
  "type": "object",
  "properties": {
    "tunerIndex": { "type": "number" }
  }
}
```

## epg.programs-updated

An event sent when EPG programs of a service are updated.

```jsonc
{
  "type": "object",
  "properties": {
    "serviceId": { "type": "number" }  // ServiceId
  }
}
```
## recording.started

An event sent when recording for a TV program is started.

```jsonc
{
  "type": "object",
  "properties": {
    "programId": { "type": "number" }  // ProgramId
  }
}
```

## recording.stopped

An event sent when recording for a TV program is stopped.

```jsonc
{
  "type": "object",
  "properties": {
    "programId": { "type": "number" }  // ProgramId
  }
}
```

## recording.failed

An event sent when recording for a TV program is failed.

```jsonc
{
  "type": "object",
  "properties": {
    "programId": { "type": "number" },  // ProgramId
    "reason": {
      "oneOf": [
        // start-recording-failed
        {
          "type": "object",
          "properties": {
            "type": { "type": "string", "const": "start-recording-failed" },
            "message": { "type": "string" },
          }
        },
        // io-error
        {
          "type": "object",
          "properties": {
            "type": { "type": "string", "const": "io-error" },
            "message": { "type": "string" },
            "osError": { "type": ["number", "null"] }
          }
        },
        // pipeline-error
        {
          "type": "object",
          "properties": {
            "type": { "type": "string", "const": "pipeline-error" },
            "exitCode": { "type": "number" }
          }
        },
        // need-rescheduling
        {
          "type": "object",
          "properties": {
            "type": { "type": "string", "const": "need-rescheduling" },
          }
        },
        // schedule-expired
        {
          "type": "object",
          "properties": {
            "type": { "type": "string", "const": "schedule-expired" },
          }
        },
        // removed-from-epg
        {
          "type": "object",
          "properties": {
            "type": { "type": "string", "const": "removed-from-epg" },
          }
        },
      ]
    }
  }
}
```

## recording.rescheduled

An event sent when recording for a TV program is rescheduled.

```jsonc
{
  "type": "object",
  "properties": {
    "programId": { "type": "number" }  // ProgramId
  }
}
```

## recording.record-saved

An event sent when a record is saved successfully.

```jsonc
{
  "type": "object",
  "properties": {
    "recordId": { "type": "string" }  // RecordId
  }
}
```

The `recording.record-saved` event for a record may be sent multiple times.  For example, a
`recording.record-saved` event will be sent when metadata of the TV program currently recorded is
updated.

The `recording.record-saved` event for a record will no long sent once the recording status of the
record becomes `finished`, `failed` or `canceled`.

The order of occurrence of `recording.started`, `recording.stopped` and `recording-record-saved`
events is not guaranteed.

## recording.record-removed

An event sent when a record is removed.

```jsonc
{
  "type": "object",
  "properties": {
    "recordId": { "type": "string" }  // RecordId
  }
}
```

## recording.content-removed

An event sent when a content is removed.

```jsonc
{
  "type": "object",
  "properties": {
    "recordId": { "type": "string" }  // RecordId
  }
}
```

## recording.record-broken

An event sent when a record has been broken.

```jsonc
{
  "type": "object",
  "properties": {
    "recordId": { "type": "string" },  // RecordId
    "reason": { "type": "string" }
  }
}
```

Like `recording.record-saved` events, the `recording.record-broken` event for a record may be sent
multiple times.

The `recording.record-broken` event for a record will no long sent once the recording status of the
record becomes `finished`, `failed` or `canceled`.

The order of occurrence of `recording.started`, `recording.stopped` and `recording-record-broken`
events is not guaranteed.

## timeshift.timeline

An event sent when the timeshift timeline for a service advances.

```jsonc
{
  "type": "object",
  "properties": {
    "recorder": { "type": "string" },
    "startTime": { "type": ["number", "null"] },  // UNIX time in milliseconds or null
    "endTime": { "type": ["number", "null"] },    // UNIX time in milliseconds or null
    "duration": { "type": "number" }              // in milliseconds
  }
}
```

## timeshift.started

An event sent when timeshift recording for a service is started.

```jsonc
{
  "type": "object",
  "properties": {
    "recorder": { "type": "string" }
  }
}
```

## timeshift.stopped

An event sent when timeshift recording for a service is stopped.

```jsonc
{
  "type": "object",
  "properties": {
    "recorder": { "type": "string" }
  }
}
```

## timeshift.record-started

An event sent when timeshift recording for a TV program is started.

```jsonc
{
  "type": "object",
  "properties": {
    "recorder": { "type": "string" },
    "recordId": { "type": "number" }
  }
}
```

## timeshift.record-updated

An event sent when timeshift recording for a TV program is updated.

```jsonc
{
  "type": "object",
  "properties": {
    "recorder": { "type": "string" },
    "recordId": { "type": "number" }
  }
}
```

## timeshift.record-ended

An event sent when timeshift recording for a TV program is ended.

```jsonc
{
  "type": "object",
  "properties": {
    "recorder": { "type": "string" },
    "recordId": { "type": "number" }
  }
}
```

## onair.program-changed

An event sent when the on-air TV program of a service is changed.

```jsonc
{
  "type": "object",
  "properties": {
    "serviceId": { "type": "number" }  // ServiceId
  }
}
```

[SSE]: https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events
