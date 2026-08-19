# IPC

A surface and the core exchange messages over a local socket. This document records the envelope those messages travel in. What a command asks for and what comes back is in [the CLI specification](cli.md); this document does not repeat it.

The `cistern-contract` crate defines the envelope in Rust. The crate is the source while every surface is written in Rust. Once one is written in another language, this document takes that place.

## Transport

| Platform | Address |
| --- | --- |
| Unix | `$XDG_RUNTIME_DIR/cistern/sock`, or `~/.local/state/cistern/sock` when that variable is unset |
| Windows | the named pipe `\\.\pipe\cistern` |

**The core runs on Unix.** The requirements say macOS or Linux, the daemon reads the vendor's limit through a pseudo-terminal, and nothing is built or run against Windows. The Windows address is written down and the code carries the arms for it so that the envelope is one document rather than two, but a Windows core is not something this project offers today. One thing already known to be missing: `interprocess` answers `Unsupported` to a receive timeout on a named pipe, and the wait on a request below is set through one.

The core creates the socket when it starts and removes it when it exits. A surface that cannot connect reports that the core is not running.

One connection carries one request. The core never sends a message the surface did not ask for.

## Framing

One message per line, terminated by `\n`.

A message is a JSON object serialized without indentation, so no unescaped newline appears inside one.

A request is one line and so is the response. A connection closed before a request arrives is answered with nothing, which is what makes connecting and leaving a way to ask whether a core is there.

## Request

```json
{"version":"0.1.0","type":"task_show","params":{"task":"2"}}
```

| Field | Type | Description |
| --- | --- | --- |
| `version` | string | The release version of the surface sending the request |
| `type` | string | The command, in snake_case |
| `params` | object | The command's arguments |

`type` is the command name with spaces replaced by underscores: `task add` is `task_add`. What `params` holds is the argument table in that command's section of the CLI specification, together with what a surface supplies on the user's behalf. `task_add` carries `cwd`, the directory the command was run in, because the core runs as a daemon and its own working directory is not the user's. Every value in `params` crosses as a string, whatever form the argument table gives it; a value of another type is refused with code 2 and never reaches the core.

## Response

```json
{"type":"task_show","data":{"id":"task:2","state":"Completed"}}
```

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | The `type` of the request being answered |
| `data` | object | The command's output fields |

What `data` holds is the output table in that command's section of the CLI specification.

## Error

```json
{"type":"error","code":3,"message":"no such task"}
```

| Field | Type | Description |
| --- | --- | --- |
| `code` | integer | An exit code from the CLI specification |
| `message` | string | What went wrong, in one sentence |
| `data` | object | Present on some errors, described where they arise |

The core decides the code and the surface exits with it.

A `type` the core does not recognise is an error with code 2.

## Version

The surface sends its release version on every request. The core refuses the request unless that version matches its own: the whole version while the major is 0, the major alone from 1.0 on.

A refusal is an error with code 5, carrying both versions.

```json
{"type":"error","code":5,"message":"core is 0.2.0, surface is 0.1.0","data":{"core":"0.2.0","surface":"0.1.0"}}
```

## The `core_version` request

```json
{"version":"0.1.0","type":"core_version","params":{}}
```

```json
{"type":"core_version","data":{"version":"0.1.0"}}
```

Answers with the core's own version. It is the one request the core answers whatever version the surface sent, so a surface can report which side is behind. No command maps to it, so it has no section in the CLI specification.
