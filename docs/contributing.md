> English translation of the Chinese document. Original: [contributing.zh-CN.md](contributing.zh-CN.md). Technical review pending.

# Development and collaboration

## 1. Environment

| Item | Version / notes |
|---|---|
| Rust | stable (1.77+; 1.97 verified locally) |
| Node.js | 20+ (24 verified locally) |
| macOS | Xcode Command Line Tools |
| Windows | WebView2 Runtime + MSVC build tools |
| Linux | Tauri's webkit2gtk dependencies; see the [Tauri docs](https://v2.tauri.app/start/prerequisites/) |

Install the Tauri CLI through npm; a global `cargo install tauri-cli` is not needed.

## 2. First run

```sh
cd apps/desktop
npm install
npm run tauri dev      # dev mode, frontend hot reload
```

Frontend only (no Tauri shell; `invoke` will fail — styling work only):

```sh
cd apps/desktop && npm run dev
```

Build:

```sh
cd apps/desktop && npm run tauri build
```

## 3. Common commands

```sh
# Rust
cargo check --workspace --all-targets
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# Smoke tests that need a local runtime (not run in CI)
cargo test -p mixum-env-runtime-apple -- --ignored
cargo test -p mixum-env-runtime-docker -- --ignored

# Frontend typecheck + build
cd apps/desktop && npm run build
```

## 4. Style

- Rust: default `rustfmt`; clippy `-D warnings` clean.
- TypeScript: `strict` + `noUnusedLocals`; no `any` (when unavoidable, use `unknown`
  and narrow it).
- Comments explain **why**, not "this line assigns"; Chinese comments stay consistent
  with mixumOS.
- Before adding a dependency, ask whether the standard library or an existing
  dependency can do it.

## 5. Cross-platform

- Platform differences are only allowed inside `crates/mixum-env-runtime-*`, using
  `cfg(target_os)`.
- Do not write `#[cfg(unix)]` in `core`.
- The WSL 2 backend must also compile on macOS/Linux (returning `Unsupported`).
- When using platform APIs such as `std::os::unix::*`, provide the non-Unix branch,
  or the Windows build breaks.

## 6. Branches and commits

- Branches: `feat/<module>-<topic>`, `fix/<module>-<topic>`, e.g. `feat/wsl2-import`.
- One PR touches only its own module; explain cross-module changes in the description.
- Commit message: `type(scope): summary`, with type `feat` / `fix` / `docs` /
  `refactor` / `test` / `chore`, e.g. `feat(docker): parse port mappings`.
- Contract changes (public API of `mixum-env-core`) get `[contract]` in the title and
  need one reviewer from each side.

## 7. Pre-commit checklist

- [ ] `cargo check --workspace --all-targets` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] `cd apps/desktop && npm run build` passes
- [ ] No `target/`, `node_modules/`, `dist/` or signing artifacts committed
- [ ] Contract or module boundary changed → update `docs/`

## 8. Known pitfalls

| Symptom | Cause | Fix |
|---|---|---|
| Finder launch cannot find docker/container | GUI process has no login-shell PATH | `lib.rs` `augment_path()` adds `/opt/homebrew/bin` and friends |
| `npm run build` cannot find the esbuild binary | npm does not run postinstall by default | `package.json` declares `allowScripts`; if needed, `npm rebuild esbuild` |
| Tauri build complains about missing icons | Icons listed in `tauri.conf.json` must exist | Icons are committed under `src-tauri/icons/` |
| New command 404s in the frontend | Handler not registered | `generate_handler!` in `lib.rs` |
| Windows build fails on a Unix-only API | Missing `cfg` branch | See section 5 |

## 9. Do not commit

- Local paths, tokens, logs;
- Personal signing configuration in `tauri.conf.json`;
- Large build artifacts (`dist/`, `target/`).
