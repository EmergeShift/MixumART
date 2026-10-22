# mixum-artd

`mixum-artd` 是宿主侧 Unix socket API。它不替代 `mixum` CLI，而是给浏览器、
MCP、IDE 或未来的 `mixum-artd` 客户端提供长期控制面。

```text
mixum-artd --socket PATH --data-dir DIR
```

默认：

```text
data:   ~/Library/Application Support/dev.mixum.agentenv/mixum-cli
socket: <data>/artd.sock
```

socket 权限为 `0600`，数据目录为 `0700`；只允许当前系统用户访问。

## 协议

一行一个 JSON 请求，一行一个 JSON 响应；不支持批量。

```json
{"id":1,"method":"env.create","params":{"title":"fix login","profile":"mixumos","runtime":"apple","image":"127.0.0.1:15000/mixumos:0.1.4"}}
```

成功：

```json
{"id":1,"ok":true,"result":{...}}
```

失败：

```json
{"id":1,"ok":false,"error":{"code":4,"message":"..."}}
```

`error.code` 与 CLI 退出码一致。

## 方法

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

实现方式：daemon 用 sibling `mixum --json ...` 执行同一套规则，不复制端口、
状态或 MAP 逻辑。这样 CLI 和 daemon 的行为不会漂移。

## 启动

```bash
cargo build -p mixum-cli --release
./target/release/mixum-artd
```

当前版本是前台进程；由 launchd/systemd 或 Agent 管理器托管。browser/serve
入口会在 daemon API 稳定后接入。
