# Account standing — synthetic native evidence

All captures use the same 1120×760 application crop, dark appearance, C.UTF-8 locale, software
Xvfb environment and isolated HOME/XDG directories. Each process was launched with
`--demo --demo-settings=storage`, scrolled to the bottom of Data & Privacy, and terminated without
using saved login storage or making a live Discord request.

- `before.png`: the `main` tree at `2e673c47`, rebuilt in the disposable baseline worktree. The
  current page has Saved Login followed directly by Your Privacy.
- `standing-ready.png`: this branch's new read-only Account Standing section before any request;
  its state is **Not checked**.
- `after.png`: after a native click on **Check account standing** using the default synthetic `good`
  fixture. The displayed **All Good** result is fixture data, not a live account assertion.
- `after-violation.png`: the same interaction with `--demo-standing=violation`, demonstrating the
  bounded classification, expiration, appeal-status and explanation-link layout. All values are
  explicit synthetic fixtures.

The group describes its normal-session GET `/safety-hub/@me` route as unofficial and unverified.
It has no appeal, disable, delete or other write action. Unknown or unavailable standing remains
unavailable and is never inferred from flags, DMs, profile errors or this visual evidence.
