# Mixum Browser Bridge — early access

> **Status: early access.** Not in the Chrome Web Store yet; developer mode only.
> **状态：早期访问。** 尚未上架 Chrome Web Store，只能开发者模式加载。
This build is for early testing before the Chrome Web Store listing is approved.
Chrome does not allow a CLI or desktop app to silently install an extension.
Two steps are always required:

1. install the Mixum host (bundled with the CLI and with MixumART);
2. load the extension manually in Chrome, or install it from the Web Store once
   the listing is live.

## Option A — CLI only

Extract the release tarball anywhere, then:

```bash
cd mixum-cli-<version>-<os>-<arch>
./bin/mixum browser install
```

This writes the Chrome native-messaging manifest and prints the extension
directory. Then:

1. Open `chrome://extensions`.
2. Enable **Developer mode**.
3. Click **Load unpacked**.
4. Select:
   ```text
   share/mixum/extensions/mixum-browser
   ```

The extension popup should show `connected to MixumART native host`.

If Chrome is not your default browser, the manifest is written to the stable
Google Chrome profile. Chromium/Chrome for Testing use a different
`NativeMessagingHosts` directory; for those, set:

```bash
MIXUM_CHROME_NATIVE_HOST_DIR="$HOME/Library/Application Support/Chromium/NativeMessagingHosts"
```

(On Linux use `~/.config/chromium/NativeMessagingHosts`.)

## Option B — MixumART

Install MixumART. It bundles the same CLI and host. On every launch, MixumART
runs the bundled `mixum browser install`; **do not install the CLI separately**.

Then load the extension manually as above. The bundled extension lives at
`MixumART.app/Contents/Resources/extensions/mixum-browser`; select that folder
in **Load unpacked**.

## What is automatic and what is not

| Step | CLI | MixumART | Notes |
|---|---|---|---|
| install `mixum-browser-host` | ✅ bundled | ✅ bundled | no separate download |
| write native-messaging manifest | ✅ `browser install` | ✅ first use | user-level file, no admin |
| load Chrome extension | ❌ manual | ❌ manual | Chrome restriction |
| share cookies/tabs | ✅ `browser export/share` | ✅ | after approval |
| Web Store install | — | — | after Google review |

## Verify

```bash
./bin/mixum browser status
```

Expected: the native host path and extension ID
`ahimdgpdmhhbhkokcablggjjidkdlnhl`.

If the popup says `native host not connected`:

- confirm `mixum-browser-host` is next to `mixum`;
- rerun `mixum browser install`;
- fully quit and reopen Chrome;
- check that the extension is loaded from the path printed by `browser install`.

## Enterprise

Managed Chrome can deploy the extension by policy instead of manual Load
unpacked. The host still comes from the CLI/MixumART bundle. Use the generated
policy fragment from:

```bash
mixum browser policy
```
