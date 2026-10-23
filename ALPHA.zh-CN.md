# Alpha 说明

MixumART 是 alpha 软件。它是我们每天在用的构建，但还不是一个打磨好的产品：API 会变，部分平台路径只是“能编译”，打包也刻意做得很轻。

这些不是值得骄傲的设计决定，只是一个没有融资的小团队今天能拿出来的东西。如果哪里硌手，欢迎开 issue——“这是 alpha”是解释，不是借口。

## macOS：应用没有签名

我们没有购买 Apple Developer Program（$99/年），所以 `.app` 既没有签名，也没有公证。首次打开会被 Gatekeeper 拦下。

- 右键应用 → **打开** → 再点一次 **打开**；或者：

  ```bash
  xattr -dr com.apple.quarantine "/Applications/mixum ART.app"
  ```

- 从源码自己构建的包也一样。

CLI、Docker 镜像和 npm SDK 不受 Apple 签名影响。

## Windows：SmartScreen，不是许可证

微软**不要求**开发者账号才能分发 Microsoft Store 之外的 `.exe`/`.msi`。“未知发布者”是 SmartScreen 信誉提示，不是许可证检查。

- 点 **更多信息** → **仍要运行**。
- 代码签名证书可以去掉这个提示，但它是付费选项，我们还没买。它在待办清单上，不在预算里。
- WSL 2 后端完全没有签名要求。

## Chrome：先用开发者模式

浏览器桥还没有上 Chrome Web Store。我们不想让一个 alpha 去排队等商店审核，所以先发源码。

1. 打开 `chrome://extensions`。
2. 打开 **开发者模式**。
3. **加载已解压的扩展程序** → 选择 `extensions/mixum-browser`。
4. Chrome 启动时会提示“开发者模式扩展”，这是预期行为。

隐私政策在 `extensions/mixum-browser/store/privacy-policy.md`；托管页面仍在搭建中。

## Homebrew

暂时没有 cask。源码构建的 formula 在计划中（[#65](https://github.com/EmergeShift/MixumART/issues/65)）；cask 要等签名并公证的 DMG。

## 为什么这么糙？

我们是一个没有融资的小团队。钱主要花在构建机器和 CI 上，还没有花到证书机构和商店审核上。如果你能帮忙做签名、公证或打包，欢迎开 issue——这是实打实的工作，不是人情。
