> English translation of the Chinese design doc. Original: [architecture.zh-CN.md](architecture.zh-CN.md). Technical review pending.

# Architecture

## 1. What this project is

mixum Agent Runtime is **the layer installed on the host**: a Tauri desktop application responsible for

1. connecting mixumOS to the container runtime the host already provides (WSL 2 / Docker / Apple Container);
2. acting as the manager for mixumOS containers (listing, lifecycle, and later image and kernel preparation);
3. taking on the interaction design that used to live inside mixumOS and should now live on the host.

mixumOS (`../mixumos`) is the foundation: `mixumd` inside the container, the mixcomp compositor, the snapshot engine, the `wss://host:7601` protocol. This repository does not reimplement them; it only consumes them.

## 2. Division of responsibilities with mixumOS

| Concern | mixumOS (foundation) | Agent Runtime (this repository) |
|--------|----------------|---------------------|
| In-container daemon | `mixum-agentd` / `mixumd` | Not involved |
| Protocol | Protocol v1 (WSS + envelope) | Connects as a client starting from M4 |
| Snapshot / task engine | Rust crates | Provides an entry point starting from M4 |
| Host runtime | Not responsible (v1 non-goal) | **Core responsibility of this repository** |
| Installation and first launch | Image / distribution build | Host installer (M4) |

> The v1 non-goals in `mixumos/docs/architecture.md` state "Windows/macOS as a container host
> (client only)". This repository is exactly the product of lifting that restriction: **the two sets of documents are maintained separately**,
> and the host form is authoritative in this repository; the mixumOS-side document awaits synchronization by its maintainers.

## 3. Process model and data flow

Single process plus one webview:

```
React (webview)
   │  invoke("list_containers", { kind })
   ▼
src-tauri/commands.rs
   │
   ▼
BackendRegistry ──▶ Arc<dyn RuntimeBackend>
                        │
                        ▼
              process::run(cli, args, timeout)   # argv passed directly, no shell
                        │
                        ▼
              docker ps / container list / wsl.exe
                        │
                        ▼
              parse → Vec<Container> (serde) ──▶ JSON ──▶ UI
```

- **Backend registration**: `build_registry()` in `lib.rs` assembles backends from `RuntimeKind::ALL`; adding a runtime means adding only one branch.
- **Command layer**: `commands.rs` only forwards parameters and converts errors to strings; zero lines of platform logic.
- **Concurrency**: commands are async, and Tauri bundles a tokio runtime; external calls all carry timeouts (5s for probing, 10s for listing), so a stuck daemon will not freeze the UI.
- **State**: probed once at startup, then refreshed manually by the user; there is no background polling.

## 4. Platform support

| Capability | macOS | Windows | Linux |
|------|-------|---------|-------|
| Apple Container | ✅ Integrated (probe/list/create/start-stop/delete) | — | — |
| Docker | ✅ Integrated (probe/list/create/start-stop/delete) | Code ready, not verified on real hardware | Code ready, not verified on real hardware |
| WSL 2 | — | ✅ Probe/list/import/start-stop/unregister | — |
| Build dependencies | Xcode CLT | WebView2 + MSVC | webkit2gtk etc. (see Tauri docs) |

All three runtimes provide write operations through the same `RuntimeBackend` contract; the UI renders according to backend state, and additional capabilities that are not implemented (for example, a graphical flow for image import) return an explicit error.

## 5. Security and boundaries

- **No privilege escalation**: all external commands run as the current user; capabilities that require administrator rights (for example, enabling the WSL feature) use the system authorization flow in M3 rather than escalating privileges inside the application.
- **No shell**: `process::run` accepts only argv and rejects concatenated command strings.
- **Timeouts**: every CLI call has an upper bound, avoiding hangs when a daemon misbehaves.
- **No network access**: M0 makes no network requests; when M4 connects to mixumd, it connects only to loopback addresses or nodes the user explicitly configures; tokens go into the system keychain and are never written to logs.
- **Least privilege**: Tauri capabilities enable only `core:default`; plugin permissions are added as needed.
- **No telemetry**.

## 6. Roadmap

| Milestone | Content | Completion criterion |
|--------|------|----------|
| **M0 (completed)** | Tauri 2 skeleton, three-backend registration, probing and listing, module ownership docs | macOS build/run verification passed |
| **M1 (completed)** | Docker write operations (create/start-stop/delete), container logs | A mixumOS container fully runs through the GUI |
| **M2 (partially completed)** | Apple Container write operations; OCI image import / kernel setup still needs a wizard | Start a node with one click |
| **M3 (completed)** | WSL 2 integration (import/start-stop/unregister) | Import a distribution on Windows and start a node |
| M4 | Host installer, Agent external connector, persistent terminal | Install a new machine end-to-end in one flow and manage nodes |

## 7. v0 non-goals

- Multi-user / multi-tenant;
- Remote node orchestration (an operator already exists on the cloud side);
- Image building (handled by mixumOS's `dist/`);
- Bare-metal (non-container) deployment.
