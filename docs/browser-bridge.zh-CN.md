# Chrome 桥

桥把**用户批准的域名**共享给容器里的 Chromium。它导出 cookie 和当前打开的标签页；
不镜像实时标签状态。

## 组件

```text
Chrome（用户配置）
  └─ MV3 扩展（稳定 ID ahimdgpdmhhbhkokcablggjjidkdlnhl）
       │ native messaging（dev.mixum.browser）
       ▼
mixum-browser-host  ── Unix socket ──  mixum browser export/share
       │
       ▼
容器：mixum-browser apply lease.json
       └─ Chromium CDP：Storage.setCookies → Target.createTarget
```

## 安装

```bash
mixum browser status   # manifest / 宿主二进制 / socket / 下一步
mixum browser install
```

这会写入 native messaging manifest。扩展本身**不会**被这个命令静默安装。

alpha 阶段先解压加载：

1. 打开 `chrome://extensions`。
2. 打开 **开发者模式**。
3. **加载已解压的扩展程序** → 选择 `extensions/mixum-browser`。
4. Chrome 启动时会警告开发者模式扩展；这是预期行为。

开发者模式是 alpha 阶段的支持路径，不是长期消费者路径。商店上架后切换到它，或用策略部署：

1. Chrome Web Store listing（用户一键安装）；
2. 企业策略 `ExtensionInstallForcelist` / `ExtensionSettings`（受管设备，无需用户操作）。

构建商店上传包：

```bash
mixum browser package --out mixum-browser-extension.zip
```

在 Chrome Web Store 开发者控制台上传这个 zip。native host manifest 仍由
`mixum browser install` 按机器安装；企业部署可以用 MDM 推送同一个 manifest。

受管 Chrome 可以生成强制安装策略片段：

```bash
mixum browser policy --out chrome-policy.json
# macOS MDM 可以改用 plist：
mixum browser policy --format plist --out chrome-policy.plist
```

它包含扩展 `ahimdgpdmhhbhkokcablggjjidkdlnhl` 的 `ExtensionSettings`，
`installation_mode=force_installed`。把它作为 Chrome 策略通过 MDM 下发；消费者安装
仍然需要 Web Store listing。

扩展请求宽泛的 host 权限，因为 `chrome.cookies` 需要它们。native broker 仍会拒绝用户
未批准的域名，lease 只包含这些域名的 cookie。公开上架前可以收窄
`optional_host_permissions`。

## 使用

```bash
# agent 发起；打印请求 id 和给人类的确切命令
mixum browser request "my env" --domain example.com --url https://example.com/app --reason "read current tab"
# 人类用宿主密码批准；成功后执行 share
mixum browser approve browser-01...
# 人类直接路径
mixum browser export --domain example.com --url https://example.com/app --json
mixum browser share "my env" --domain example.com --url https://example.com/app
```

`share` 把一次性 lease 上传到容器，调用 controller，然后删除 lease 文件。controller
用每容器独立的 profile 启动 Chromium，在打开标签页**之前**注入 cookie，并且从不打印
cookie 值。

容器需要 Chromium 和 `mixum-browser` 二进制。在母体里装一次：

```bash
mixum base shell my-base
pacman -S chromium
# 把 mixum-browser 复制/安装到 /usr/local/bin
mixum base commit my-base --tag v1
```

## 安全

- `--url` 是标签范围过滤器（精确或前缀）；cookie 仍是域名范围，因为 cookie API 没有
  path/URL 隔离。URL lease 并不让域名 cookie 在快照里变得安全。
- cookie 是凭据；lease 默认只读。
- lease 限定在已批准域名，apply 后删除。
- 浏览器 profile 是每容器独立的，不能进入 `.mart`/CRIU 快照。恢复或迁移后重新
  share，不要复制 profile。
- controller 在容器内运行。root 容器可以在 lease 存在期间读取它；这是人类批准 root
  容器风险的一部分。
- 跨容器共享意味着对每个容器用同样的已批准域名调用一次 `share`；没有全局 cookie
  store。
