# Alpha notes

MixumART is alpha software. It is the build we use every day, but it is not a
polished product: APIs move, some platform paths are only code-complete, and the
packaging is deliberately minimal.

None of this is a design decision we are proud of. It is what an unfunded team
can ship today. If a rough edge blocks you, open an issue — "this is alpha" is an
explanation, not an excuse.

## macOS: the app is unsigned

We have not bought the Apple Developer Program ($99/year), so the `.app` is
neither signed nor notarized. Gatekeeper will refuse the first launch.

- Right-click the app → **Open** → **Open** again, or run:

  ```bash
  xattr -dr com.apple.quarantine "/Applications/mixum ART.app"
  ```

- A build from source has the same caveat.

The CLI, Docker images, and the npm SDK are not affected by Apple signing.

## Windows: SmartScreen, not a license

Microsoft does **not** require a developer account to distribute an `.exe`/`.msi`
outside the Microsoft Store. The "Unknown publisher" dialog is SmartScreen
reputation, not a license check.

- Click **More info** → **Run anyway**.
- A code-signing certificate would remove the warning, but it is a paid option we
  have not bought yet. It is on the list, not in the budget.
- The WSL 2 backend has no signing requirement at all.

## Chrome: developer mode, for now

The browser bridge is not in the Chrome Web Store. We did not want to put an
alpha through store review, so the source is the distribution.

1. Open `chrome://extensions`.
2. Enable **Developer mode**.
3. **Load unpacked** → select `extensions/mixum-browser`.
4. Chrome warns about developer-mode extensions on startup. That is expected.

The privacy policy lives in
`extensions/mixum-browser/store/privacy-policy.md`; the hosted page is still
under construction.

## Homebrew

There is no cask yet. A source-build formula is planned in
[#65](https://github.com/EmergeShift/MixumART/issues/65); a cask waits for a
signed and notarized DMG.

## Why so rough?

We are a small, unfunded team. The money we have goes to build machines and CI
minutes, not to certificate authorities and store listings. If you can help with
signing, notarization or packaging, open an issue — it is real work, not a favor.
