> English translation of the Chinese document. Original: [daemon.zh-CN.md](daemon.zh-CN.md). Technical review pending.

# mixum-artd

> **Status: partial.** `mixum-artd` exists, but its public command surface is not exposed yet.
> `mixum-artd` is the host-side Unix socket API. It does not replace the `mixum` CLI;
> it gives browsers, MCP, IDEs or future `mixum-artd` clients a long-lived control plane.

```text
mixum-artd --socket PATH --data-dir DIR
```

Defaults:

```text
data:   ~/Library/Application Support/dev.mixum.agentenv/mixum-cli
socket: <data>/artd.sock
```

The socket is `0600`, the data directory `0700`; only the current OS user can connect.

## Protocol

One JSON request per line, one JSON response per line; batching is not supported.

```json
{"id":1,"method":"env.create","params":{"title":"fix login","profile":"mixumos","runtime":"apple","image":"127.0.0.1:15000/mixumos:0.1.4"}}
```

Success:

```json
{"id":1,"ok":true,"result":{...}}
```

Failure:

```json
{"id":1,"ok":false,"error":{"code":4,"message":"..."}}
```

`error.code` matches the CLI exit code.

## Methods

| method | params |
| --- | --- |
| `doctor` | `{}` |
| `runtime.list` | `{}` |
| `env.list` | `{all?: bool}` |
| `env.create` | `{title, method?, profile?, image?, runtime?, port?, noStart?, waitMap?}` |
| `env.show` | `{selector}` |
| `env.start` | `{selector}` |
| `env.stop` | `{selector}` |
| `env.remove` | `{selector, force?}` |
| `env.windows` | `{selector, timeout?}` |
| `env.logs` | `{selector, tail?}` |
| `snapshot.list` | `{selector}` |
| `snapshot.create` | `{selector, label?}` |
| `port.check` | `{port}` |
| `port.list` | `{}` |
| `config.show` | `{}` |
| `config.set` | `{key, value}` |
| `config.unset` | `{key}` |

Implementation: the daemon runs the sibling `mixum --json ...` binary, so the same
rules apply; it does not copy port, state or MAP logic. CLI and daemon behavior
therefore cannot drift.

## Running

```bash
cargo build -p mixum-cli --release
./target/release/mixum-artd
```

The current version is a foreground process, managed by launchd/systemd or an agent
supervisor. The browser/serve entry points will connect once the daemon API is stable.
