> English translation of the Chinese document. Original: [module-map.zh-CN.md](module-map.zh-CN.md). Technical review pending.

# Module map: where each piece of code belongs

This page answers one question: **where should the thing I want to change live?**

Find the matching row here before changing code; if there is no row, add one to
`docs/` first, then write the code.

## 1. Directory layout

```
MixumART/
├── Cargo.toml                      # Rust workspace: members, shared dependency versions
├── crates/
│   ├── mixum-env-core/             # domain types + backend contract (the only shared Rust code)
│   │   └── src/{container,error,process,runtime}.rs
│   ├── mixum-env-runtime-docker/   # Docker backend
│   ├── mixum-env-runtime-apple/    # Apple Container backend (macOS)
│   └── mixum-env-runtime-wsl2/     # WSL 2 backend (Windows, parallel group)
├── apps/
│   └── desktop/
│       ├── src-tauri/              # Tauri shell: commands, state, windows, permissions
│       │   ├── src/lib.rs          # assemble backends + register commands + PATH fix
│       │   ├── src/commands.rs     # all #[tauri::command]
│       │   ├── capabilities/       # Tauri v2 capabilities (only core:default by default)
│       │   └── tauri.conf.json
│       ├── src/                    # React UI
│       │   ├── App.tsx             # sidebar + top bar + view switching
│       │   ├── api.ts              # the single invoke exit
│       │   ├── types.ts            # TS mirror of core types
│       │   ├── status.ts           # status copy/colour mapping
│       │   ├── views/              # page-level components
│       │   └── components/         # reusable small components
│       └── package.json
├── docs/
└── README.md
```

## 2. Dependency direction (one-way only)

```
apps/desktop/src  ──invoke──▶  apps/desktop/src-tauri
                                      │
                                      ▼
                            crates/mixum-env-core
                                      ▲
                    ┌─────────────────┼─────────────────┐
        runtime-docker          runtime-apple        runtime-wsl2
```

Hard rules:

- `mixum-env-core` **does not depend on** any runtime crate, nor on Tauri.
- runtime crates **do not depend on each other**.
- The Tauri layer does not call `Command::new` directly; all external calls go
  through the backend crates.
- The frontend never writes command-name strings; it goes through `src/api.ts`.

## 3. Ownership table

| Path | Responsible for | Who can change it | Must not appear |
|---|---|---|---|
| `crates/mixum-env-core` | domain types (`Container`/`CreateSpec`/`PortMapping`), `RuntimeBackend` contract, `RuntimeStatus`, `EnvError`, timed command calls | everyone; **contract changes need review from both groups** | CLI args, `cfg(target_os)` branches, Tauri types, UI copy |
| `crates/mixum-env-runtime-docker` | `docker` CLI probing, listing, output parsing; (M1) write operations | this group | React/Tauri deps, other runtime code |
| `crates/mixum-env-runtime-apple` | `container` CLI probing, listing, output parsing; (M2) write operations | this group | same as above |
| `crates/mixum-env-runtime-wsl2` | `wsl.exe` interaction, distro management | **WSL 2 group** | same as above; do not change core to accommodate an implementation |
| `apps/desktop/src-tauri/src/commands.rs` | commands: fetch state, map errors, return to frontend | this group | business logic, platform details |
| `crates/mixum-env-surface` | surface/frame/input wire types and codecs | display group | platform/CLI/Tauri types |
| `crates/mixum-env-compositor` | in-container Wayland compositor, buffer/damage/seat | display group | host UI, Tauri deps |
| `crates/mixum-env-display` | host TCP debug client, window composition, input return | display group | Tauri private state |
| `crates/mixum-env-map` | MAP WSS/TOFU + display.list/attach/input | display group | platform/CLI details |
| `apps/desktop/src-tauri/src/lib.rs` | backend assembly, command registration, startup environment fix | this group | business logic |
| `apps/desktop/src/views/` | pages (environments, containers, later images/settings) | frontend | command names, backend private fields |
| `apps/desktop/src/components/` | stateless small components (badges etc.) | frontend | data requests |
| `apps/desktop/src/{api,types,status}.ts` | contract mirror and display mapping | frontend + core in sync | business branches |
| `docs/` | ownership, contracts, build notes | everyone | — |

> "Who can change it" is not permission; it is **conflict cost**. If you change a
> directory owned by another group, explain why in the PR and ask the owner to take
> a look before merging.

## 4. Where common changes go

| What I want to do | Where to change |
|---|---|
| Add a runtime (e.g. Podman) | add a `RuntimeKind` variant in `core/runtime.rs` → create `crates/mixum-env-runtime-podman` → add a branch in `build_registry` in `lib.rs` → add a row to `docs/` |
| Add a field to the container list (e.g. CPU/memory) | `Container` in `core/container.rs` → each backend parses its own → `src/types.ts` → table column |
| Add a container operation (start/stop/delete) | define semantics in `docs/runtime-contract.md` first → implement the trait method in backend crates → `commands.rs` → `api.ts` → UI |
| Add a Tauri command | `commands.rs` → `generate_handler!` in `lib.rs` → wrap in `api.ts` |
| Add a page | `src/views/` → Tab in `App.tsx` → add a view row to `docs/ui.md` |
| Change status copy | display only → `src/status.ts`; protocol field → `core/runtime.rs` (contract process) |
| Add an error kind | `core/error.rs` + the error mapping table in `docs/runtime-contract.md` |
| Handle a platform difference | inside the corresponding runtime crate, with `cfg`; **not** in core |
| Upgrade a dependency | Rust: root `Cargo.toml` `[workspace.dependencies]`; frontend: `apps/desktop/package.json` |
| Add build/release scripts | create `scripts/` (none yet) |

## 5. Parallel collaboration rules (WSL 2 group)

- The WSL 2 group only changes `crates/mixum-env-runtime-wsl2/**` and its own docs.
- That crate is already registered in the GUI and shows "connecting" on Windows, so
  integration testing does **not** need this group.
- If the contract is insufficient (missing field/error/trait method):
  1. Update `docs/runtime-contract.md` with the motivation first.
  2. Put `[contract]` in the PR title.
  3. Get one reviewer from this group and one from the WSL 2 group before merging.
- Keep changes to shared files (root `Cargo.toml`, `lib.rs`, `types.ts`) small; this
  group merges conflicts.
- Suggested branch names: `feat/wsl2-import`, `fix/docker-parse`; one PR covers only
  its own module.

## 6. Contract change checklist

When changing the public API of `mixum-env-core`, confirm each item:

- [ ] `docs/runtime-contract.md` updated
- [ ] all three backends compile (including Windows)
- [ ] `src/types.ts` synced
- [ ] serialization has unit-test coverage (field names are part of the contract)
- [ ] when an old backend does not support it yet, use `Option` + `serde(default)`;
      do not delete fields
