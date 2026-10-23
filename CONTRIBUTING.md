# Contributing to MixumART

MixumART is the host-side runtime for mixumOS. Keep platform differences inside
`crates/mixum-env-runtime-*`; do not add `cfg(target_os)` to `mixum-env-core`.

## Before you start

- Read [`docs/architecture.md`](docs/architecture.md) and
  [`docs/runtime-contract.md`](docs/runtime-contract.md).
- Security issues: see [`SECURITY.md`](SECURITY.md). Do not open a public issue.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd apps/desktop
npm ci && npm run build
```

New Tauri commands must be registered in `generate_handler!` and go through
`src/api.ts` on the frontend.

## Pull requests

- One logical change per PR.
- Add a test where the behavior can be tested.
- Update `docs/` for user-visible changes.
- No CLA; contributions are accepted under Apache-2.0.
