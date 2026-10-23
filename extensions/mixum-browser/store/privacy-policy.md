# Mixum Browser Bridge privacy policy

Mixum Browser Bridge runs locally in Chrome.

- It reads cookies and tabs only after the local native host asks for a
  user-approved URL/domain.
- The native host is a local process; the extension has no analytics, remote
  logging, or third-party endpoints.
- Cookie values are held in memory, handed to the local Mixum runtime, and are
  not written to logs or snapshots.
- Approval/audit metadata contains the URL/domain, timestamp, and outcome; it
  does not contain cookie values.
- Removing the extension stops all access. The local Mixum runtime deletes its
  lease files after applying them.

Contact: security@emergeshift.com


Privacy policy URL: https://mixum.emergeshift.com/privacy/browser-bridge (under construction; see #64)
