# Web Store submission

Build the upload zip:

```bash
mixum browser package --out dist/mixum-browser-extension.zip
```

The zip contains only `manifest.json`, `background.js`, `popup.html`, and
`popup.js`; native-host templates and stale zips are excluded.

Before submitting:

1. Host `privacy-policy.md` at a public HTTPS URL and put it in the listing.
2. Add at least one 1280x800 screenshot and the 128x128 store icon.
3. Use the stable extension ID `ahimdgpdmhhbhkokcablggjjidkdlnhl` (the manifest
   `key` is already present).
4. Submit for review; consumer installs then use the Web Store, managed
   installs use `mixum browser policy`.
