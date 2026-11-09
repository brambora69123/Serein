# Interactive profile editing — synthetic native evidence

Before and after use the same `target/debug/serein --demo --demo-settings=profile` startup, 1120×760
application crop, C.UTF-8 locale, software Xvfb environment and isolated HOME. `before.png` was
captured from current `main`; `after.png` was captured from this branch. No saved login, account
storage or live Discord write was used. All profile values are synthetic.

The after view adds the banner editor and its truthful disabled state on the fixture's free account.
It also keeps Save/Cancel below a separately bounded scrolling region, so the new controls do not push
those actions out of reach. The UI's Nitro check affects only new banner selection; removal remains
available. Other currently unimplemented cosmetics are not represented as supported here.

For the interaction check, the isolated X11 window was focused and the preview's display name was
clicked, followed by ordinary key events. `preview-focus.png` shows the resulting change in both the
editor and preview, the focus outline and the enabled fixed-footer state. This verifies native input
routing without saving anything: the synthetic session was then terminated without clicking Save.

Protocol/media tests and the mock HTTP sparse-write/readback tests complement these captures. They do
not establish live Discord interoperability. Account-standing, connections, plugins and privacy work
are separate work packages and are not claimed by this evidence set.
