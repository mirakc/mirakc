# Events

mirakc provides the Web endpoint `/events` for event notifications using
Sever-Sent Events ([SSE]).

Using this feature, users can implement useful functions such as a rule-based
automatic recording scheduler like [this](https://github.com/mirakc/contrib/blob/main/recording/simple-rules.js).

## epg.programs-updated

An event sent when EPG programs of a service are updated.

```json5
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
