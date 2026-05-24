# SJBIS TODO

## Active

- [x] Make text rail the default
- [x] In the text rail agent card, make it a button/filter so clicking filters to JUST that agent instead of muting
- [x] Make keyboard shortcut indicator (`.k` / `kbd`) bigger and brighter
- [x] Add optional custom note to answers (returned to calling agent)
- [ ] iMessage plugin / side-script for SJBIS
  - Phase 1: macOS Notification Center observer (`com.apple.MobileSMS` distributed notifications)
  - Phase 2: Full agent wrapper (persistent daemon, maintains state, uses `sjbis ask --blocking` to send answers back)
  - Core challenge: deduplication logic (same text shouldn't spawn 5 questions)
  - Core challenge: filtering heuristic (only surface actual questions, not "on my way" or "lol")
