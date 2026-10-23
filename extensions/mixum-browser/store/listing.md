# Chrome Web Store listing

## Name
Mixum Browser Bridge

## Short description (max 132 chars)
Share only the cookies and tabs you approve with a Mixum container browser. Values stay local and are never logged.

## Detailed description
Mixum Browser Bridge connects your Chrome profile to a Mixum container browser
without exporting a full cookie database or asking you to load an unpacked
extension.

When a Mixum environment requests a URL or domain, you approve that exact
request in MixumART (or the CLI fallback). Only then does the extension send the
matching cookies and tabs to the local Mixum native host. The host hands the
lease to the container's Chromium through CDP. Lease files are deleted after
use, cookie values are never logged, and nothing is sent to a remote server.

The extension does not read cookies or tabs in the background. It waits for an
explicit native-host request and filters by the approved URL/domain prefix.

## Category
Productivity

## Language
English

## Single purpose
Move user-approved cookies and open tabs from the user's Chrome profile into a
local Mixum container browser.

## Permission justification
- `cookies`: read the cookies matching the user-approved domain so the container
  browser can open the same authenticated page.
- `tabs`: copy only the approved URL/tab into the container.
- `tabGroups`: preserve the approved tab's group when copying it.
- `nativeMessaging`: talk to the local Mixum native host; no network server.
- `storage`: hold the pending approval nonce locally.
- `host_permissions` `http://*/*`, `https://*/*`: match the user-approved URL;
  the extension is inert until a native-host request arrives.

## Privacy policy URL
https://mixum.emergeshift.com/privacy/browser-bridge (**under construction**; tracking: [#64](https://github.com/EmergeShift/MixumART/issues/64))
