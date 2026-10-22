---
name: mixum-art
description: Create, inspect and monitor MixumOS agent environments through the `mixum` CLI or `mixum mcp` tools.
---

# MixumART environments

Use `mixum` to create and inspect MixumOS environments. Prefer the MCP tools
when the client supports MCP; otherwise use the CLI and `--json`.

## Rules

- Use `--profile mixumos` for environments that need MAP windows.
- Use `--profile generic` only for tools/build containers; generic containers
  have no agentd, no MAP and no window stream.
- Never ask the user to paste an admin token. `mixum` reads it locally and
  never prints it.
- Do not run `docker run` / `container run` directly when MixumART integration
  is required; use `mixum` so ports, naming and MAP registration stay consistent.
- Do not expose the terminal through MCP; terminal is a privileged local path.
- Snapshot restore is destructive and requires `--yes` on the CLI.

## CLI

```bash
mixum env create --profile mixumos --title "fix login" --method subagent
mixum env list --json
mixum env windows "fix login" --json
mixum env show "fix login"
mixum env start "fix login"
mixum env stop "fix login"
mixum env logs "fix login" --tail 100
mixum env open "fix login" --window 1 --no-open
```

`mixum env open` starts a read-only local browser monitor and prints a URL;
it does not grant input or terminal access.

When a profile has multiple local images, pass `--image` explicitly. If more
than one runtime is ready, pass `--runtime docker|apple|wsl2`.

## Desktop

`mixum desktop` opens the installed MixumART app (macOS bundle id
`dev.mixum.agentenv`). On Windows/Linux set `MIXUM_DESKTOP_APP` to the app
executable.

## MCP

Run `mixum mcp` as a stdio MCP server. Available tools:

- `mixum_env_list`
- `mixum_env_create`
- `mixum_env_show`
- `mixum_env_windows`
- `mixum_env_start`
- `mixum_env_stop`
- `mixum_snapshot_list`
- `mixum_snapshot_create`
- `mixum_port_check`

Example `mixum_env_create` arguments:

```json
{
  "title": "fix login",
  "method": "subagent",
  "profile": "mixumos",
  "runtime": "apple"
}
```

The result contains the environment id, MAP address, TLS fingerprint and
window count. It never contains the token.
