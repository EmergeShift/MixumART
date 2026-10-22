# Chrome bridge

The bridge shares **user-approved domains** with a container Chromium. It exports
cookies and currently open tabs; it does not mirror live tab state.

## Components

```text
Chrome (user profile)
  └─ MV3 extension (stable ID ahimdgpdmhhbhkokcablggjjidkdlnhl)
       │ native messaging (dev.mixum.browser)
       ▼
mixum-browser-host  ── Unix socket ──  mixum browser export/share
       │
       ▼
container: mixum-browser apply lease.json
       └─ Chromium CDP: Storage.setCookies → Target.createTarget
```

## Install

```bash
mixum browser install
```

This writes the native messaging manifest. The extension itself is **not**
silently installed by this command. Production distribution is one of:

1. Chrome Web Store listing (one click for users);
2. enterprise policy `ExtensionInstallForcelist` / `ExtensionSettings` (managed
   devices, no user action).

Loading an unpacked extension is a development-only path. Do not tell end users to
enable developer mode; publish the extension or deploy it by policy.

Build the store upload artifact:

```bash
mixum browser package --out mixum-browser-extension.zip
```

Upload that zip at the Chrome Web Store developer console. The native host
manifest is still installed per machine by `mixum browser install`; enterprise
deployment can push the same manifest with MDM.

For managed Chrome, generate the force-install policy fragment:

```bash
mixum browser policy --out chrome-policy.json
```

It contains `ExtensionSettings` for extension
`ahimdgpdmhhbhkokcablggjjidkdlnhl` with `installation_mode=force_installed`.
Feed it through MDM as a Chrome policy; consumer installs still require the
Web Store listing.

The extension requests broad host permissions because `chrome.cookies` needs them.
The native broker still refuses any domain the user did not approve, and the lease
contains only those domains' cookies. Narrow `optional_host_permissions` can be
added before a public store release.

## Use

```bash
# agent asks; prints a request id and the exact human command
mixum browser request "my env" --domain example.com --url https://example.com/app --reason "read current tab"
# human approves with host password; share runs on success
mixum browser approve browser-01...
# direct human path
mixum browser export --domain example.com --url https://example.com/app --json
mixum browser share "my env" --domain example.com --url https://example.com/app
```

`share` uploads a one-shot lease to the container, calls the controller, and
deletes the lease file. The controller starts Chromium with a per-container
profile, injects cookies **before** opening tabs, and never prints cookie values.

The container needs Chromium and the `mixum-browser` binary. Install them once in
a base:

```bash
mixum base shell my-base
pacman -S chromium
# copy/install mixum-browser into /usr/local/bin
mixum base commit my-base --tag v1
```

## Security

- `--url` is a tab-scope filter (exact or prefix); cookies remain domain-scoped
  because the cookie API has no path/URL isolation. A URL lease does not make
  domain cookies safe to show in a snapshot.
- Cookies are credentials; leases are read-only by default.
- A lease is scoped to approved domains and deleted after apply.
- The browser profile is per-container and must not be included in `.mart`/CRIU
  snapshots. After restore or migration, re-share instead of copying the profile.
- The controller runs inside the container. A root container can read the lease
  while it exists; that is part of the root-container risk the human approves.
- Cross-container sharing means calling `share` once per container with the same
  approved domains; there is no global cookie store.
