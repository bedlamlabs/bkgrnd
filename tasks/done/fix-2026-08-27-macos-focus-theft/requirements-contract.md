# Requirements Contract — fix-2026-08-27-macos-focus-theft: Prevent macOS menu-bar focus theft
Approved: 2026-08-28T00:25:53Z

## What we are building
| # | Feature | Acceptance criteria (user-observable) |
|---|---------|----------------------------------------|
| 1 | Non-stealing menu-bar activation | Opening bkgrnd from its menu-bar icon no longer replaces or repeatedly reclaims the application the user was working in. |
| 2 | Working menu-bar panel | The bkgrnd panel still opens from the tray, remains usable, and dismisses normally without adding a Dock icon or menu bar. |

## How each feature will be verified
| # | Local proof | Production proof | Dixie routes | API probes | Data checks |
|---|-------------|------------------|--------------|------------|-------------|
| 1 | A macOS source guard proves the forced Tao focus path is absent; the full Rust suite remains green. | The installed app is opened through its real menu-bar accessibility element while another app is frontmost; System Events confirms bkgrnd does not become frontmost. | N/A — native macOS windowing behavior. | N/A | System Events frontmost-process state and installed `LSUIElement` value. |
| 2 | The full Tauri Rust suite plus the menu-bar interaction harness. | The installed panel is toggled open and closed through the real status item and the app remains running as a background UI element. | N/A — native macOS windowing behavior. | N/A | Process/window visibility and installed bundle metadata. |

## Deploy plan
Target: `/Applications/bkgrnd.app` plus `bedlamlabs/bkgrnd` through the repository delivery workflow. Monitor: CI plus installed-app process and native focus probes. Rollback: restore the pre-hotfix bundle retained by the existing macOS installer.

## Stop conditions
No playback, server, mobile, or unrelated UI changes. Escalate only for failed deployment authentication, an unavailable deployment host/network, destructive production action outside replacing the explicitly requested app build, or a new product requirement outside preventing involuntary focus theft.
