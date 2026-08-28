# Prevent macOS menu-bar focus theft

## Type
hotfix

## What
bkgrnd's menu-bar panel now opens without replacing the application the user is working in, while preserving normal panel opening and dismissal.

## Why
The prior tray interaction forced macOS application activation, which could leave bkgrnd frontmost and make other applications unable to retain focus during memory pressure.

## Changes
- Removed the forced application-focus request from the menu-bar panel toggle.
- Preserved background UI-element registration, panel positioning, visibility, dismissal, and process lifetime.
- Added installed-app acceptance coverage for frontmost-application retention and panel toggling.

## Verification

| What Was Changed | Method | Evidence |
|------------------|--------|----------|
| Menu-bar activation retains the user's frontmost application | API | evidence/production-verification.json |
| The installed background panel opens and dismisses without terminating bkgrnd | API | evidence/production-verification.json |
