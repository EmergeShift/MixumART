> English translation of the Chinese document. Original: [cli.zh-CN.md](cli.zh-CN.md). Technical review pending.

# mixum CLI

`mixum` is the local control-plane CLI for MixumART. It manages MixumOS environments directly through the Docker / Apple Container / WSL 2 backends, does not depend on a Tauri window, and does not use the `mart` command name.

```text
mixum          用户/Agent CLI
mixum-artd     后续常驻宿主控制面（本版本尚未暴露命令）
mixumd         容器内 agentd，保持原名
mixcomp        容器内 compositor，保持原名
```

## Stable command surface

```text
mixum doctor
mixum runtime list
mixum env list [--all]
mixum env create --profile mixumos --title TITLE [--method subagent] [--image IMAGE]
mixum env show SELECTOR
mixum env start SELECTOR
mixum env stop SELECTOR
mixum env remove SELECTOR [--force]
mixum env windows SELECTOR [--timeout 30]
mixum env open SELECTOR [--window ID] [--port 0] [--no-open]
mixum config show
mixum config set KEY VALUE
mixum config unset KEY
mixum env logs SELECTOR [--tail 200]
mixum env terminal SELECTOR [--shell /bin/sh]
mixum env snapshot list SELECTOR
mixum env snapshot create SELECTOR [--label LABEL]
mixum env snapshot restore SELECTOR NODE --yes
mixum port check PORT
mixum port list
mixum desktop
mixum mcp
mixum version
```

All commands support the global `--json`. Unimplemented features do not appear in the command table and do not return a “TODO/Unsupported” placeholder.

## Data directory

Priority:

1. `--data-dir`
2. `MIXUM_DATA_DIR`
3. `dev.mixum.agentenv/mixum-cli` under `$XDG_DATA_HOME` / `~/Library/Application Support`

Files:

```text
state.json      环境标题、method、runtime、端口、指纹；权限 0600
state.lock      create 的互斥锁
```

If `state.json` is corrupted, the CLI reports an error directly instead of silently rebuilding it; this is to avoid losing environment ownership information.

## Environment list

By default, `mixum env list` shows only environments that are managed or whose image name contains `mixumos`, to avoid mixing build/normal containers into the Agent environment list. Only `--all` lists all containers in the runtime.

## Runtime selection

`doctor` / `runtime list` displays the three backends.

- When `mixum env create` does not specify `--runtime`:
  - macOS prefers Apple Container;
  - Windows prefers WSL 2;
  - other platforms prefer Docker;
  - if the preferred backend is unavailable, use the next ready backend.
- When `--runtime` is specified, that backend must be ready; otherwise an error is returned.
- When multiple backends are ready at the same time, the creation output explicitly records the actual backend.

## Profile and image

### `--profile mixumos`

Requires the image reference to contain `mixumos` and the image to exist locally. The image must include:

- `/entrypoint.sh`
- `mixumd` (agentd)
- `mixcomp`
- `7601/tcp`

When `--image` is omitted:

- only one local mixumos image → use it;
- multiple → report an error, list the candidates, and require an explicit `--image`;
- none → report an error and prompt to pull/load first.

### `--profile generic`

An explicit `--image` is required. MAP and windows are not guaranteed; only the container is created, and MAP is not awaited by default. Suitable for build/tool containers.

## Ports

- The MAP port inside the container is fixed at `7601/tcp`.
- The host publishes only to `127.0.0.1` and does not expose it to the LAN.
- By default, select the first port in `17600-17999` that can be bound and is not occupied by a mixum environment.
- `--port N` uses the specified port; if the port is occupied or published by another environment, a conflict is returned.
- `--port 0` is equivalent to automatic allocation.
- `state.lock` is held during creation to prevent two concurrent create operations from selecting the same port.
- `mixum port check N` only checks binding on the local `127.0.0.1`; `port list` lists ports already published by the runtime.

## Selector

`SELECTOR` can be:

- full container id
- container name
- title in `state.json`
- prefix of id/name (at least 4 characters)

On ambiguity, an error is returned with candidates listed; it does not guess.

## Errors and exit codes

| code | Meaning |
| --- | --- |
| 0 | success |
| 1 | IO / state file error |
| 2 | argument error |
| 3 | environment/image not found |
| 4 | conflict (port, duplicate name, removal while running, ambiguity) |
| 5 | runtime unavailable or command failed |
| 6 | MAP/agentd failure |

On failure in `--json` mode, stdout outputs:

```json
{"error":{"code":4,"message":"..."}}
```

Successful output is command-specific JSON; tokens never appear in the output.

## JSON examples

### create

```json
{
  "environment": {
    "id": "mixum-cli-probe-01m5h9",
    "name": "mixum-cli-probe-01m5h9",
    "title": "CLI probe",
    "runtime": "apple_container",
    "image": "127.0.0.1:15000/mixumos:0.1.4",
    "state": "running",
    "method": "subagent",
    "managed": true,
    "mapPort": 17777,
    "mapAddr": "127.0.0.1:17777",
    "fingerprint": "50:d2:..."
  },
  "mapReady": true,
  "fingerprint": "50:d2:...",
  "windows": 0
}
```

### windows

```json
{
  "environment": "mixum-cli-probe-01m5h9",
  "fingerprint": "50:d2:...",
  "windows": [
    {"id": 1, "title": "foot", "appId": "foot", "width": 300, "height": 169}
  ]
}
```

## Security

- admin-token is read from the container through the local runtime `exec`, used on demand, and not written to CLI state;
- the CLI does not output tokens; `env logs` redacts `map1.…`;
- MAP uses TOFU + fingerprint pinning; the first fingerprint is written to known-hosts for `mixum-env-map`;
- the terminal uses the host `exec -it` and is a local privileged path; the CLI allows only an interactive TTY;
- this is still the admin-token approach; certificate/public-key authentication is not yet implemented; do not write tokens into images or snapshots.

## Config

```bash
mixum config set default-runtime apple
mixum config set default-image 127.0.0.1:15000/mixumos:0.1.4
mixum config show --json
```

`default-runtime` (docker/wsl2/apple) and `default-image` are configurable. `env create` uses these defaults when `--runtime` / `--image` are not passed; file permissions are 0600.

## MCP

`mixum mcp` starts a stdio MCP server using the same CLI implementation; tools do not return tokens.

```text
mixum_env_list
mixum_env_create
mixum_env_show
mixum_env_windows
mixum_env_start
mixum_env_stop
mixum_snapshot_list
mixum_snapshot_create
mixum_snapshot_restore
mixum_snapshot_fork
mixum_snapshot_export
mixum_snapshot_import
mixum_window_screenshot
mixum_window_click
mixum_window_move
mixum_window_type
mixum_window_key
mixum_window_scroll
mixum_window_focus
mixum_port_check
```

Screenshot returns MCP image content. Restore requires `confirm: "RESTORE"`.
Terminal and arbitrary shell remain CLI-only; human approval commands
(`mixum privilege approve/deny`) are intentionally not exposed to the agent.
The companion skill is at `skills/mixum-art/SKILL.md`.

## Base environments

```bash
mixum base create NAME --image IMAGE
mixum base shell NAME
mixum base commit NAME --tag v1
mixum base list
mixum env create --title TITLE --base NAME:v1
mixum env rebase SELECTOR --base NAME:v2
```

Apple Container builds from the builder rootfs tar with `FROM scratch`/`ADD`
(native, no Docker). Docker uses the same rootfs tar. WSL2 exports a rootfs tar
and uses it as the image.

## Chrome bridge

```bash
mixum browser install
mixum browser export --domain example.com --json
mixum browser share SELECTOR --domain example.com
```

Production extension distribution is Chrome Web Store or enterprise policy;
`install` writes the native host manifest, not a silent unpacked extension.
See [browser-bridge.md](browser-bridge.md).

## Privilege approval

```bash
mixum privilege watch SELECTOR
mixum privilege approve SELECTOR REQUEST_ID
mixum privilege deny SELECTOR REQUEST_ID
```

`approve` asks for the host computer password via `sudo -v`; the agent's sudo
wrapper only submits one argv and never gets a root shell. See
[privilege.md](privilege.md).

## Daemon

The host-side Unix socket API is `mixum-artd`, in the same crate as the CLI:

```bash
cargo build -p mixum-cli --release
./target/release/mixum-artd
```

It uses the sibling `mixum --json` to enforce the same rules; socket permissions are 0600. For the protocol, see [docs/daemon.md](daemon.md).

## Build

```bash
cargo build -p mixum-cli --release
./target/release/mixum --help
```

The current version provides `mixum mcp` (stdio) and `mixum desktop`. The `mixum serve` browser UI is not yet implemented and does not ship as an empty shell command first.
