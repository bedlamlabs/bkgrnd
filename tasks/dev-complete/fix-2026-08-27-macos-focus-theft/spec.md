# Prevent macOS menu-bar focus theft

## Reproduction

The installed app is correctly registered with `LSUIElement=true`, but opening its panel invokes Tauri/Tao window focus. Tao 0.34.6 implements that call with `makeKeyAndOrderFront` followed by `activateIgnoringOtherApps:YES`. Pre-reboot RunningBoard evidence showed a `frontmost` assertion held for roughly 77 seconds during memory pressure.

## Fix

Remove the forced macOS focus request from the tray toggle. Preserve positioning, display, loss-of-focus dismissal, accessory activation policy, and background-app registration. Do not replace the forced request with another API that ignores the current frontmost application.

## Verification

First prove the existing source fails a guard against the forced focus path. After implementation, run the focused harness and complete Rust suite. Build and install the real macOS application, programmatically click its actual status item through Accessibility while another application is frontmost, and prove bkgrnd does not acquire frontmost status. Toggle it again and confirm the background process remains healthy.
