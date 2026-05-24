# SJBIS TODO

## Active

- [x] Make text rail the default
- [x] In the text rail agent card, make it a button/filter so clicking filters to JUST that agent instead of muting
- [x] Make keyboard shortcut indicator (`.k` / `kbd`) bigger and brighter
- [x] Add optional custom note to answers (returned to calling agent)
- [ ] iMessage plugin / side-script for SJBIS
  - [x] Phase 1: SQLite DB poller + question filter + dedup cache
  - [x] Phase 1: Signed .app bundle for isolated Full Disk Access
  - [x] Phase 1: JXA (JavaScript for Automation) message fetcher with JSON parsing
  - [x] Phase 2: AppleScript/JXA reply sending back to iMessage contacts
  - [ ] Phase 1: macOS NSDistributedNotificationCenter observer (deferred — JXA poller works)
  - [ ] Phase 2: End-to-end test with real Messages (waiting for user to grant Full Disk Access)
  - Core challenge: deduplication logic (same text shouldn't spawn 5 questions)
  - Core challenge: filtering heuristic (only surface actual questions, not "on my way" or "lol")
- [x] Add sjbis status <id> command to check notification state after blocking timeout
- [x] Support escape sequences (\n, \t) in --detail and --question
- [x] Support multi-line detail rendering with <br> tags
- [x] Add --detail-markdown flag with rich text rendering (bold, italic, lists, links, code)
