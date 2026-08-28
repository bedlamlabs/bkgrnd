# Delivery Report — Prevent macOS menu-bar focus theft (1 task)
Status: DELIVERED

| # | Feature | What it does now | How we verified it | Evidence |
|---|---------|------------------|--------------------|----------|
| 1 | Non-stealing menu-bar activation | Opening bkgrnd from its menu-bar icon leaves the application the user is working in frontmost. | The installed menu-bar item was clicked while Safari was frontmost, and Safari remained frontmost after bkgrnd opened. | evidence/production-verification.json; evidence/acceptance-test-production.json |
| 2 | Working menu-bar panel | The bkgrnd panel still opens and dismisses normally as a background UI element without terminating the app. | The installed panel was opened and dismissed through its real menu-bar item, its background registration was confirmed, and its process remained running. | evidence/production-verification.json; evidence/acceptance-test-production.json |

Drill-down: evidence/gate-cards/, evidence/review/, run-ledger.md
