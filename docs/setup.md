# Setup and agent MCP registration

`mixum setup` is the shared first-run flow for the CLI and the MixumART desktop
app. Both render the same plan from `mixum-setup` and write the same
`setup.json` / `config.json` files under the mixum data directory.

```bash
mixum setup
mixum setup --lang en --runtime docker --image mixumos:0.1.6 --agent codex
mixum setup --dry-run --json
```

## Steps

1. **Language** — `zh-CN` or `en`. The choice is saved and reused by later
   setup runs.
2. **Runtime** — Docker, Apple Container or WSL 2. The planner marks platform
   combinations that cannot work (for example Apple Container on Windows or
   WSL 2 on macOS) as unsupported. A missing runtime shows an install command;
   a stopped runtime shows a start command.
3. **mixumOS** — use an existing local image, an existing environment
   (`--environment ID` in the CLI), or a local/remote image reference. `--pull` (CLI) or the pull checkbox (desktop)
   fetches the image before registration.
4. **Agents** — register the `mixum mcp` stdio server in the selected host
   clients. Existing entries are updated; unrelated settings are preserved and
   a `.mixum.backup` file is kept next to changed configuration.

## Supported host clients

| ID | Client | Shape |
|---|---|---|
| `claude_code` | Claude Code | `~/.claude.json`, `mcpServers` |
| `claude_desktop` | Claude Desktop | platform `claude_desktop_config.json`, `mcpServers` |
| `codex` | Codex | `~/.codex/config.toml`, `[mcp_servers.mixum]` |
| `cursor` | Cursor | `~/.cursor/mcp.json`, `mcpServers` |
| `windsurf` | Windsurf | `~/.codeium/windsurf/mcp_config.json`, `mcpServers` |
| `gemini_cli` | Gemini CLI | `~/.gemini/settings.json`, `mcpServers` |
| `vscode` | VS Code | user `mcp.json`, `servers` with `type: "stdio"` |
| `opencode` | OpenCode | user `opencode.json`, `mcp` with a command array |

The registered command is the absolute `mixum` executable followed by `mcp`;
the desktop chooses its bundled CLI when present, otherwise `mixum` from
`PATH`.

For privileged runner installation or start actions, the desktop shows an
in-app confirmation first, then opens the native OS authorization UI:
macOS uses `osascript` administrator privileges, Windows uses a UAC
`Start-Process -Verb RunAs`, and Linux uses `pkexec`. No terminal is needed.

## Re-entry

The desktop app opens Setup on first launch and keeps an **初始化** tab plus a
Settings button afterwards. Rerunning either UI or `mixum setup` only adds or
updates the selected agent entries; it does not recreate an existing
environment or overwrite unrelated settings.

## Update notice

Normal `mixum` commands (including `mixum setup`) perform a cached, best-effort
GitHub release check and print a notice on stderr when a newer version exists.
Set `MIXUM_NO_UPDATE_CHECK=1` to disable it. JSON output and the stdio MCP
server do not receive the notice.
