# Selective upstream sync — synthetic visual evidence

Baseline: detached `79ac72a6c7340920f09da39ee92e5fbe54d76fe2`. Candidate: final PR #2 head.
Both revisions used identical synthetic state, logical window sizes, theme and input sequence. The
examples were temporary debug-only native eframe fixtures with no account, authentication worker,
Discord connection, microphone, camera, screen capture or persistent cache. All captures use the
host's Vulkan renderer; no account or private desktop pixels are included.

| Pair | Synthetic reproduction / result |
|---|---|
| `voice-before.png` / `voice-after.png` | A deliberately long room name. Before, multiple text lines overlap participant rows. After, the room name is constrained to one ellipsized row; full hover/accessibility text remains available. |
| `profile-before.png` / `profile-after.png` | The same full-profile popout and locally generated effect metadata, with a uniform translucent coverage layer to expose corner overflow. Before shows the old square-corner spill. After clips it to the rounded outline. A pixel comparison confines the change to the top corner at y=333–352. `rounded_effect_mesh_removes_square_corners_and_stays_inside_the_card` also checks every vertex and the removed area. |
| `search-before.png` / `search-after.png` | The same offline result and right-click. Before offers only Jump. After opens the result-scoped Copy message / Copy message link / Copy message ID menu, with no unrelated timeline-message controls. |

The preview limitation remains separate from these images: `discord_protocol` tests distinguish an
oversized notice from literal notice text, while UI fixtures cover plugin dispatch, selection ownership
and Shift+F10 anchoring.

No valid hidden-channel before/after pair was captured. Agent device access is disabled in the host,
and the temporary notification/guild fixture did not consistently reach the intended public dialog
state. Failed images are omitted, not presented as matching evidence. The offline
`hidden_voice_participants_are_cached_only_with_hidden_channels_opted_in` test instead verifies both
empty admission with hidden channels disabled and the exact three synthetic participants with them
enabled; core roster and selected-stage access tests cover retention without granting entry or exposing
an inaccessible selected stage. Additional mixed-roster and invite tests ensure guild-rail hover and
voice-invite cards also filter retained entries by current channel visibility.
