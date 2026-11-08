# Weavelo Air V3 implementation and device evidence

2026-11-08. V3 was reviewed against the preservation catalog and implemented in the original GPUI desktop and Android Compose products. The concrete interaction contracts are in `docs/UI_UX_SPECIFICATION.md` and `docs/MOBILE_UI_UX_DESIGN.md`.

## Review decisions

- Preserve secondary functions through More and existing drawers: source selection, files, audio, clipboard, quality, control ownership, AI and session management. Four visible primary controls are not a limit on product functionality.
- Keep desktop controls outside video; move persistent network telemetry to More → Display. Fullscreen controls require intentional activation.
- Desktop phone navigation uses Back, Home and Fit. Do not present a fake remote rotation operation; retain source-driven orientation adaptation.
- Mobile desktop viewing uses one four-icon capsule and shows shortcut keys with the keyboard.
- Mobile phone viewing uses a thin visible handle in a real external touch gutter. Its four-icon capsule includes More, which retains Disconnect and secondary functions. Back does not implicitly disconnect.

## Delivered and checked

Desktop build 20261108.155 is installed on Mac Studio, MacBook, HO5 and cube. Android versionCode 109 is installed on Redmi and Nubia. Existing signing identities and membership were preserved. Source changes are on main; the installer scope correction is commit `77fdbde2b24a36a4f24f9cd2c7edc69e50d6f0fe`. Desktop 155 source is `a68ea84761458e809f3622f3713669f048c29f57`; Android 109 source is `e82346e190e94da4bdc076963f903bb6e10f023a`.

Relevant Kotlin, input, window geometry, source-browser and button-style checks passed. The final release regression entry passed 1053 tests with 25 skips. This is functional regression evidence, not universal physical acceptance.

Actual executable evidence is retained under `.rust-storage/validation-logs/weavelo-air-20261108/`:

| Observation | Evidence |
| --- | --- |
| Light desktop management; white primary-button text | `studio155-management.png`, `after154-book-management.png` |
| Device, file and network entry interactions | `studio155-devices.png`, `studio155-transfers.png`, `studio155-pairing.png` |
| Desktop receiving phone pixels with outer navigation and no statistics overlay | `book155-phone-window0.png`, remote `book155-phone-proof.json` |
| Mobile receiving a real Mac fixture; keyboard input recorded | `redmi108-input-result.png`, fixture logs |
| Source picker remains usable with IME | `redmi108-source-ime.png` |
| Landscape layout and Back preserving the session | `redmi108-landscape.png`, `redmi108-back-controls.png` |
| Final phone-to-phone four-icon capsule, no duplicate top menu | `redmi109-phone-controls.png` |
| Home reaches the remote phone system desktop | `redmi109-phone-home.png` |
| Direct touch reaches the remote fixture (COUNT 0 → COUNT 1) | `redmi109-phone-fixture.png`, `redmi109-phone-input2.png` |

The 108 evidence predates the narrow 109 phone-capsule cleanup; it is not relabeled as a fresh 109 run. Screenshot pixel presence is distinct from decode/submission counters.

## Remaining coverage

This iteration does not establish exhaustive physical acceptance across every DPI, fullscreen transition, multi-touch cancellation, sensor rotation, audio route, file transfer, or long-duration performance condition. Landscape testing used a temporary Android display-rotation setting restored afterward, not physical sensor motion. No new OS consent was granted. NAS publication is not included in these installation receipts. Unrelated audio work and design input files remain untouched.
