# Mixum 浏览器桥 — 早期访问

> **状态：早期访问。** 尚未上架 Chrome Web Store，只能开发者模式加载。

这个构建用于 Chrome Web Store 审核通过前的早期测试。Chrome 不允许 CLI 或桌面应用静默
安装扩展，所以始终需要两步：

1. 安装 Mixum 宿主（CLI 和 MixumART 都自带）；
2. 在 Chrome 里手动加载扩展，或等商店上架后从 Web Store 安装。

## 方案 A — 只用 CLI

把 release tarball 解压到任意位置，然后：

```bash
cd mixum-cli-<version>-<os>-<arch>
./bin/mixum browser install
```

它会写入 Chrome native-messaging manifest，并打印扩展目录。然后：

1. 打开 `chrome://extensions`；
2. 打开 **开发者模式**；
3. 点 **加载已解压的扩展程序**；
4. 选择：
   ```text
   share/mixum/extensions/mixum-browser
   ```

扩展弹窗应显示 `connected to MixumART native host`。

如果 Chrome 不是默认浏览器，manifest 会写到稳定的 Google Chrome 配置目录。
Chromium/Chrome for Testing 使用不同的 `NativeMessagingHosts` 目录；对它们设置：

```bash
MIXUM_CHROME_NATIVE_HOST_DIR="$HOME/Library/Application Support/Chromium/NativeMessagingHosts"
```

（Linux 上用 `~/.config/chromium/NativeMessagingHosts`。）

## 方案 B — MixumART

安装 MixumART。它自带同一套 CLI 与宿主。每次启动时 MixumART 会运行自带的
`mixum browser install`；**不要再单独安装 CLI**。

然后按上面的步骤手动加载扩展。自带的扩展位于
`MixumART.app/Contents/Resources/extensions/mixum-browser`；在 **加载已解压的扩展程序**
里选择这个目录。

## 哪些是自动的，哪些不是

| 步骤 | CLI | MixumART | 说明 |
|---|---|---|---|
| 安装 `mixum-browser-host` | ✅ 自带 | ✅ 自带 | 不需要单独下载 |
| 写入 native-messaging manifest | ✅ `browser install` | ✅ 首次使用 | 用户级文件，不需要管理员 |
| 加载 Chrome 扩展 | ❌ 手动 | ❌ 手动 | Chrome 的限制 |
| 共享 cookies/tabs | ✅ `browser export/share` | ✅ | 批准之后 |
| 从 Web Store 安装 | — | — | 等 Google 审核 |

## 验证

```bash
./bin/mixum browser status
```

预期：native host 路径和扩展 ID
`ahimdgpdmhhbhkokcablggjjidkdlnhl`。

如果弹窗显示 `native host not connected`：

- 确认 `mixum-browser-host` 与 `mixum` 在同一目录；
- 重新运行 `mixum browser install`；
- 完全退出并重新打开 Chrome；
- 确认扩展是从 `browser install` 打印的路径加载的。

## 企业部署

受管理的 Chrome 可以通过策略部署扩展，而不必手动“加载已解压”。宿主仍来自
CLI/MixumART 自带包。使用生成的策略片段：

```bash
mixum browser policy
```
