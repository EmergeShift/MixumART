# Security Policy

MixumART runs on the user's desktop and holds MAP tokens, container-runtime
access and a local daemon socket.

## Reporting a vulnerability

Please **do not open a public issue**.

- Email: **security@emergeshift.com**
- Or use GitHub private vulnerability reporting: **Security** tab → **Report a vulnerability**.

Include the affected version or commit, reproduction steps and impact.

## Scope

- the Tauri desktop app and its Tauri commands/capabilities
- `mixum` CLI, `mixum-artd`, and the MCP server
- the runtime backends (Apple Container, Docker, WSL 2)
- the MAP client and surface/frame handling

## Known limitations

- The desktop currently ships with a permissive Tauri CSP; harden it before
  handling untrusted content in the webview.
- The daemon socket is user-only (`0600`) but has no additional authentication.
- Host runtime CLIs are invoked as argv, never through a shell.
