# Web Store submission

> **Alpha:** the extension is not in the Chrome Web Store yet. Until the listing
> exists, install it unpacked: `chrome://extensions` → **Developer mode** →
> **Load unpacked** → `extensions/mixum-browser`. The rest of this file is the
> submission checklist for when we do publish it.



Build the upload zip:

```bash
mixum browser package --out dist/mixum-browser-extension.zip
```

The zip contains only `manifest.json`, `background.js`, `popup.html`, and
`popup.js`; native-host templates and stale zips are excluded.

Before submitting:

1. Host `privacy-policy.md` at a public HTTPS URL and put it in the listing.
2. Use `screenshot-1280x800.png` (actual popup showing connected to the native host) and `icon-128.png`.
3. Use the stable extension ID `ahimdgpdmhhbhkokcablggjjidkdlnhl` (the manifest
   `key` is already present).
4. Submit for review; consumer installs then use the Web Store, managed
   installs use `mixum browser policy`.
