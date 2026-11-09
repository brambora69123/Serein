# Complete upstream integration — synthetic visual evidence

`after.png` is a fresh capture of the integrated native application in an isolated Xvfb display.
The application ran `target/debug/serein --demo` with an isolated home under `target/demo-home`, no
saved login, no network account and no microphone, camera or screen-capture session. The image is
cropped to the 1120×760 application window, excluding the unused virtual-display area. It verifies
that the merged timeline, channel visibility markers, member list and composer render rather than
merely compile. Locale is the synthetic fixture's Czech system locale; the binary's `0.1.0` label
is package metadata, not the upstream revision.

The existing `../extension-context-menus/before.png` and `after.png` provide the same-scenario native
comparison for the imported context-menu feature, originally captured with the upstream fixture.
`../upstream-sync-20261109/README.md` separately records search/menu, profile and voice evidence;
`../safe-equicord-ecosystem/` covers the fork's composer tools and presentation options. Those
fixtures and this integration-window capture are synthetic/offline evidence, not proof of live
Discord interoperability, catalog availability or extension-policy enforcement on a live account.
