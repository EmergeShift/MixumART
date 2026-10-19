# 开发与协作

## 1. 环境要求

| 项 | 版本 / 说明 |
|----|-------------|
| Rust | stable（1.77+，本机验证 1.97） |
| Node.js | 20+（本机验证 24） |
| macOS | Xcode Command Line Tools |
| Windows | WebView2 Runtime + MSVC 生成工具 |
| Linux | Tauri 的 webkit2gtk 等依赖，见 [Tauri 文档](https://v2.tauri.app/start/prerequisites/) |

Tauri CLI 通过 npm 安装，不需要全局 `cargo install tauri-cli`。

## 2. 首次运行

```sh
cd apps/desktop
npm install
npm run tauri dev      # 开发模式，前端热更新
```

只跑前端（不启动 Tauri 壳，`invoke` 会失败，仅用于调样式）：

```sh
cd apps/desktop && npm run dev
```

构建：

```sh
cd apps/desktop && npm run tauri build
```

## 3. 常用命令

```sh
# Rust
cargo check --workspace --all-targets
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# 需要本机 runtime 的冒烟测试（不跑 CI）
cargo test -p mixum-env-runtime-apple -- --ignored
cargo test -p mixum-env-runtime-docker -- --ignored

# 前端类型检查 + 构建
cd apps/desktop && npm run build
```

## 4. 代码风格

- Rust：`rustfmt` 默认配置；clippy `-D warnings` 干净；
- TypeScript：`strict` + `noUnusedLocals`；不用 `any`（确需时写 `unknown` 再收窄）；
- 注释写**为什么**，不写「这行在赋值」；中文注释与 mixumOS 保持一致；
- 新增依赖先问：能否用标准库或已有依赖解决。

## 5. 跨平台

- 平台差异只允许出现在 `crates/mixum-env-runtime-*` 内部，用 `cfg(target_os)`；
- 不要在 `core` 里写 `#[cfg(unix)]`；
- WSL 2 后端必须在 macOS / Linux 上也能编译（返回 `Unsupported`）；
- 使用 `std::os::unix::*` 等平台 API 时，必须有对应的非 Unix 分支，否则 Windows 构建会红。

## 6. 分支与提交

- 分支：`feat/<module>-<topic>`、`fix/<module>-<topic>`，例如 `feat/wsl2-import`；
- 一个 PR 只动自己模块；跨模块改动在描述里写明原因；
- 提交信息：`type(scope): 摘要`，type 用 `feat` / `fix` / `docs` / `refactor` / `test` / `chore`；
  例如 `feat(docker): 解析端口映射`；
- 契约改动（`mixum-env-core` 的公开 API）标题加 `[contract]`，需要两组各一人 review。

## 7. 提交前检查

- [ ] `cargo check --workspace --all-targets` 通过
- [ ] `cargo test --workspace` 通过
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 干净
- [ ] `cd apps/desktop && npm run build` 通过
- [ ] 没有提交 `target/`、`node_modules/`、`dist/`、签名产物
- [ ] 改动了契约或模块边界 → 同步 `docs/`

## 8. 已知坑

| 现象 | 原因 | 处理 |
|------|------|------|
| Finder 启动后探测不到 docker/container | GUI 进程没有登录 shell 的 PATH | `lib.rs` 的 `augment_path()` 已补 `/opt/homebrew/bin` 等前缀 |
| `npm run build` 报 esbuild 找不到二进制 | npm 默认不跑 postinstall | `package.json` 已声明 `allowScripts`；必要时 `npm rebuild esbuild` |
| Tauri 编译报缺少图标 | `tauri.conf.json` 列出的图标必须存在 | 图标已提交在 `src-tauri/icons/` |
| 新增命令后前端 404 | 忘了注册 | `lib.rs` 的 `generate_handler!` |
| Windows 构建失败在 unix-only API | 缺少 `cfg` 分支 | 见第 5 节 |

## 9. 不要提交

- 本机路径、令牌、日志；
- `tauri.conf.json` 里的个人签名配置；
- 大体积构建产物（`dist/`、`target/`）。
