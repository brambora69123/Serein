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

## Authorized applications

`connections-before.png` is the same `main` baseline's existing Connections page at 1120×760.
`authorized-apps.png` opens the new page with `--demo-settings=authorized apps` and two explicit
synthetic grants. It preserves unknown verification, scope and approval values rather than filling
them in. `authorized-app-confirm.png` follows a native Details → Deauthorize sequence: the dialog
names only Synthetic Music and displays its bounded Terms/Privacy links behind Serein's normal
external-link handling. `authorized-app-deauthorized.png` confirms that single local demo action;
the updated list retains only Synthetic Tools and reports its confirmed reconciliation. No token,
OAuth callback, client credential or live application was accessed.

## Owned profile cosmetics

`profile-before.png` shows the `main` profile editor without owned-item selectors. The after
captures use `--demo --demo-settings=profile --demo-show-cosmetics`, an explicit demo-only artwork
preference added for visual coverage. Production cosmetic-display preferences remain opt-in and
unchanged. All profile captures use the same 1440×1040 application crop and two downward wheel
steps except the open selector and save interaction, which start at the top of the page.

- `profile-cosmetics.png`: the decoration, nameplate and effect selectors plus the deliberately
  disabled profile-frame control and its unsupported-mutation-field explanation.
- `profile-options.png`: the decoration selector with a free synthetic owned item and a
  subscription-origin item marked `requires active full Nitro` on the fixture's free account.
- `profile-owned-save.png`: selecting the free **Synthetic orbit**, then Save. The avatar decoration
  and profile effect are rendered locally, the state is saved in preview, and no write reaches
  Discord. The preview honors Serein's existing cosmetic-display and clipping behavior.

The source tree and these fixtures passed `cargo xtask check` (1,688 passed, 0 failed, 14 ignored)
under two Cargo jobs and two test threads. Mock HTTP confirms ordered sparse writes and readback;
images and fixtures are not evidence of live Discord interoperability or subscription eligibility.
