# Documentation map

English is the canonical language. Once a page is translated, the English text
lives at `docs/<name>.md` and the Chinese counterpart at
`docs/<name>.zh-CN.md`. Until then the Chinese page is the only text and is
linked below.

## Pages

| Page | Status | English | 中文 |
|---|---|---|---|
| [architecture](docs/architecture.md) | current | [English](docs/architecture.md) | [中文](docs/architecture.zh-CN.md) |
| [base-env](docs/base-env.md) | current | pending | [中文](docs/base-env.md) |
| [browser-bridge](docs/browser-bridge.md) | current | pending | [中文](docs/browser-bridge.md) |
| [browser early access](docs/browser-early-access.md) | early access | [English](docs/browser-early-access.md) | pending |
| [CLI](docs/cli.md) | current | pending | [中文](docs/cli.md) |
| [contributing](docs/contributing.md) | current | pending | [中文](docs/contributing.md) |
| [CRIU build](docs/criu-build.md) | current | pending | [中文](docs/criu-build.md) |
| [daemon](docs/daemon.md) | partial | pending | [中文](docs/daemon.md) |
| [display](docs/display.md) | current (B0/B1/B2) | pending | [中文](docs/display.md) |
| [module map](docs/module-map.md) | current | pending | [中文](docs/module-map.md) |
| [privilege](docs/privilege.md) | current | pending | [中文](docs/privilege.md) |
| [runtime contract](docs/runtime-contract.md) | current | pending | [中文](docs/runtime-contract.md) |
| [snapshot limits](docs/snapshot-limits.md) | verified | pending | [中文](docs/snapshot-limits.md) |
| [support matrix](docs/support-matrix.md) | current | pending | [中文](docs/support-matrix.md) |
| [UI](docs/ui.md) | current | pending | [中文](docs/ui.md) |

## Docs-first / not implemented

- **browser early access** — the extension is not in the Chrome Web Store; it is
  loaded in developer mode. This is an alpha path, not the long-term consumer
  path.
- **mixum-artd** — the daemon exists, but its public command surface is not
  exposed yet.
- **B-route display** — B0/B1/B2 are implemented; the design notes are still
  useful, but treat the platform support matrix as the boundary.
- **CRIU build / snapshot limits** — verified notes, not a promise that every
  host or kernel combination works.

If a page says a feature exists, the code or tests should be the tie-breaker.
