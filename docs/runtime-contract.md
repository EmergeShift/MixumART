> English translation of the Chinese document. Original: [runtime-contract.zh-CN.md](runtime-contract.zh-CN.md). Technical review pending.

# Runtime backend contract

All runtime backends share one contract. **It defines the boundary for parallel
development**: the WSL 2 group implements its own `RuntimeBackend`, and this group
does not need to change the GUI.

Code: `crates/mixum-env-core/src/runtime.rs`.

## 1. Contract

```rust
#[async_trait]
pub trait RuntimeBackend: Send + Sync {
    fn kind(&self) -> RuntimeKind;
    async fn probe(&self) -> RuntimeStatus;
    async fn list_containers(&self) -> Result<Vec<Container>, EnvError>;

    // The four below have default M0 implementations: return EnvError::Unimplemented.
    async fn create_container(&self, spec: CreateSpec) -> Result<Container, EnvError>;
    async fn start_container(&self, id: &str) -> Result<(), EnvError>;
    async fn stop_container(&self, id: &str) -> Result<(), EnvError>;
    async fn remove_container(&self, id: &str) -> Result<(), EnvError>;
}
```

The write operations have M0 defaults that return `EnvError::Unimplemented`. **A
backend must compile even when it does not implement them**, so the UI can grow first.

## 2. `probe` semantics

`probe` is a status query, not an error channel:

| Return | Meaning | Typical trigger |
|---|---|---|
| `Ready { version, detail }` | CLI exists and the daemon is healthy | `docker version` succeeds; `container system status` reports `running` |
| `NotInstalled { hint }` | executable not on PATH | `ENOENT` |
| `NotRunning { hint }` | tool exists, service is down | Docker Desktop not started, `container system` not started |
| `Unsupported { reason }` | platform unsupported, or backend not wired | WSL 2 on macOS; placeholder implementation |

Hard requirements:

- **Never return `Err`**: a dead daemon is a state, not an exception.
- **No panic**, no blocking longer than 5 seconds.
- No side effects; do not modify any configuration.
- `hint` is the user's next action (e.g. "run container system start"), not a log line.

## 3. Method semantics

| Method | Success | Failure |
|---|---|---|
| `list_containers` | returns containers **including stopped ones**; empty `Vec` when none | CLI missing → `NotInstalled`; unparsable output → `Parse`; non-zero exit → `CommandFailed` |
| `create_container` | returns the created summary (state may be `created`) | duplicate name, missing image etc. map to `CommandFailed` (finer error types in M1) |
| `start_container` | container enters `running` | idempotency: **already running counts as success** |
| `stop_container` | container enters `stopped` | idempotency: **already stopped counts as success** |
| `remove_container` | container is gone | idempotency: **absent counts as success** |

The point of the idempotency rules: UI retries and double clicks must not produce
errors the user cannot understand.

WSL 2 field mapping, write timeouts and extra idempotency details are in §9.

## 4. External command rules

Always go through `mixum_env_core::process::run`:

```rust
process::run("docker", &["ps", "-a", "--no-trunc", "--format", "{{json .}}"], LIST_TIMEOUT).await
```

- Pass argv only, **never through a shell**; user input must not be concatenated into
  the command.
- Timeouts: 5 s for probe, 10 s for list; WSL 2 create 10 min (rootfs tar can be GB
  scale), start/stop/remove 30 s; timeout returns `EnvError::Timeout`.
- `stdin` is always closed, so a child cannot wait for input.
- A missing executable returns `NotInstalled`; the backend adds the install hint.
- **Encoding**: `process::run` decodes UTF-8; console tools such as wsl.exe print
  UTF-16LE, so use `process::run_raw` (raw bytes) and decode in the backend (see the
  wsl2 backend's `decode_console` heuristic).

## 5. Error mapping

`EnvError` → command-layer string → frontend error bar. The `Display` impl decides
the copy.

| Error | Trigger | What the user sees |
|---|---|---|
| `Unsupported` | platform mismatch / not wired | grey badge + reason (`probe`) |
| `NotInstalled` | `ENOENT` | red badge + install command (`probe`) |
| `Timeout` | deadline exceeded | error bar: command + seconds |
| `CommandFailed` | non-zero exit | error bar: command + first stderr line |
| `Parse` | output format changed | error bar: hint that the runtime may have upgraded |
| `Io` | process failed to start | error bar |
| `Unimplemented` | skeleton placeholder | error bar (should not be reachable from the UI before M1) |

`probe` never returns an error; a stopped service appears as
`RuntimeStatus::NotRunning` (orange badge + start hint).

## 6. Serialization contract (shared frontend/backend)

- `RuntimeKind` uses **snake_case** strings: `docker` / `wsl2` / `apple_container`.
- Struct fields use **camelCase** (`createdAt`, `readOnly`).
- `RuntimeStatus` uses `state` as the discriminant:

```json
{
  "kind": "apple_container",
  "name": "Apple Container",
  "status": { "state": "ready", "version": "1.5.0", "detail": null }
}
```

```json
{
  "id": "map-node",
  "name": "map-node",
  "image": "127.0.0.1:15000/mixumos:fix1",
  "state": "running",
  "status": null,
  "createdAt": "2026-10-18T13:02:44Z",
  "ports": [{ "host": 17601, "container": 7601, "protocol": "tcp" }]
}
```

The frontend mirror is `apps/desktop/src/types.ts`; **field names are part of the
contract** — update this document before renaming.

## 7. Implementing a new backend

1. Create `crates/mixum-env-runtime-<name>/`, depending on `mixum-env-core`.
2. Implement `RuntimeBackend` and expose
   `pub fn backend() -> Arc<dyn RuntimeBackend>` from
   `crates/mixum-env-runtime-<name>/src/lib.rs`.
3. Add a variant to `RuntimeKind` in `mixum-env-core/src/runtime.rs` (`ALL` and `id()`).
4. Add a branch to `build_registry()` in `apps/desktop/src-tauri/src/lib.rs`.
5. Add a row to the ownership table in `docs/module-map.md`.

A platform-specific backend (WSL 2) must still compile on other platforms:
`probe` returns `Unsupported`.

## 8. Testing requirements

- **Pure parsing functions must have unit tests**, with samples taken from real CLI
  output (see the two backends' `mod tests`).
- Serialization field names are locked by unit tests.
- End-to-end checks that need a local runtime are `#[ignore]`d and do not block CI:

```sh
cargo test --workspace
cargo test -p mixum-env-runtime-apple -- --ignored   # needs a local container service
cargo test -p mixum-env-runtime-docker -- --ignored  # needs a local docker daemon
cargo test -p mixum-env-runtime-wsl2 -- --ignored    # needs local WSL
```

## 9. WSL 2 write operations (parallel group, 2026-10-19)

WSL 2 has no "container" layer: one mixumOS node = one `mixum-*` distro imported from
a rootfs tar with `wsl --import`. Field mapping and semantics therefore differ from
Docker; **this section is authoritative**.

| Method | Equivalent CLI | Timeout |
|---|---|---|
| `create_container` | `wsl --import <name> <installDir> <rootfs.tar>` | 10 minutes |
| `start_container` | `wsl -d <id> -e /bin/true` | 30 s |
| `stop_container` | `wsl --terminate <id>` | 30 s |
| `remove_container` | `wsl --unregister <id>` | 30 s |

`CreateSpec` field mapping on WSL 2:

| Field | Meaning |
|---|---|
| `name` | distro name: `mixum-` prefix + `[A-Za-z0-9._-]` suffix (naming rules below) |
| `image` | **local rootfs path** — WSL has no image registry; nodes come from mixumOS `dist/` output; `.tar` and `.tar.gz` are supported |
| `ports` / `mounts` / `env` / `command` | no equivalent; **non-empty is an error**, never silently ignored |

Naming rules (list filtering, write protection and install-directory joining share
one `validate_name`, so "listed but not clickable" cannot happen):

- the prefix must be `mixum-` (case-insensitive); `mixum` / `mixumfoo` are not nodes;
- the suffix is non-empty and only `[A-Za-z0-9._-]`; separators, `..` or a trailing
  `.` are rejected — the name is joined into the install directory, so allowing them
  would allow path traversal.

Install location: `%MIXUM_WSL_INSTALL_ROOT%\<name>`; if unset,
`%LOCALAPPDATA%\mixum\wsl\<name>`, created if absent. A failed import removes that
directory, otherwise a same-name retry is blocked by a non-empty directory; `remove`
also cleans it up.

Hard requirements:

- **Only touch distros with the `mixum-` prefix**: `start` / `stop` / `remove` reject
  any other name, so the host's own `Ubuntu-24.04` / `docker-desktop` cannot be
  terminated or unregistered.
- Non-Windows host: `probe` returns `Unsupported`, and the other methods return
  `EnvError::Unsupported`; do not fall through to `NotInstalled` ("wsl.exe not found").
- `remove` deletes `ext4.vhdx` and is **irreversible**; the UI must confirm twice.
- Idempotency: `stop` / `remove` treat an absent distro as success; `stop` treats an
  existing but stopped distro as success (check state before `--terminate`);
  `create` treats a duplicate name as failure; `start` on an absent distro is an error.
- `start` only brings the distro instance up (`wsl -d <id> -e /bin/true`), temporarily.
  **WSL returns to `Stopped` about 15 s after the last process exits**; `Running` means
  "there is an active session now". Resident services (`mixumd` / entrypoint) get a new
  contract in a later milestone. Until then, the UI's "running" copy should say this is
  temporary.
- "Exists" is determined by parsing `wsl --list --verbose`, not by guessing error text.
- Parsing anchors on "the last column is the version", not a hard-coded STATE enum: a
  new WSL state (e.g. `Uninstalling`) still lists the row, and display degrades to
  `Unknown`.
- "No distros at all" is not an error: `list_containers` returns an empty `Vec`.

### Resource methods (settings page / environment terminal)

| Method | WSL 2 implementation | Why |
|---|---|---|
| `images` | empty list | WSL has no image registry; nodes come from a local rootfs tar, and after import there is no "image" layer |
| `disk_usage` | `N nodes · X GiB` + install root | WSL has no `system df`; `ext4.vhdx` is the only thing reflecting real usage |
| `logs` | `journalctl -u mixumd …`, falling back to `/var/log/mixumd.log` / `dmesg` | the node uses the systemd journal; prefer the mixumd service log, return `Err` if reading fails (log implementation merged in #34) |

These three used to fall back to the trait default (`Unimplemented`), so the settings
page showed a healthy WSL 2 backend as "capability not implemented". **"No such
concept" and "read failed" must be distinguished**: the former returns empty/zero, the
latter returns `Err`.
