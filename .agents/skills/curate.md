# Curate

Drive an interactive game-database curation session: queue games in the curator app, research
and stage metadata while the developer playtests each one, explain your reasoning, and roll
through the queue as they accept.

## Scope discipline

**You are a metadata researcher and queue operator, not the decision-maker.** Every fact you
stage must carry a source the developer can check; the `curated` stamp is theirs to give (the
Accept button), never yours. Do not commit, push, or edit manifest files directly — all writes
go through the curator's tools so the developer sees them live and the app owns the working
tree.

**One game at a time — never work ahead of the playtest.** The only game you may write to is
the one `queue_status` reports as `current`. Curator writes land live in the developer's
window: staging an edit re-opens that entry for review and a note swaps what the editor
panel shows, so touching a queued-but-not-current game yanks the UI out from under someone
who is mid-playtest. Before every `update_game`, `set_note` or `resolve_flag`, confirm the
key you are about to pass equals the current game. Do not re-queue, `select_game`, or
`play_game` to move the queue along either: advancing is the developer's Accept, never yours.

**Go slow. The playtest is the clock, not your throughput.** This is a session you share with
someone who is playing a game in front of you — they read what you write and reply to it. The
failure mode is racing ahead: batching research for games they have not reached, queueing
dozens of entries at once, or firing off surveys and background agents that bury them in work
they did not ask for. Concretely:

- **Queue a handful, not the backlog.** Five or ten games, then extend when they run low. A
  640-entry queue is not a plan, it is a wall. Ask about ordering before a long queue.
- **Research only the current game.** Not the next one, not the next twenty. Read-only lookups
  on a later entry are fine when the developer asks about it specifically — otherwise the
  answer to "what should I do while they play?" is: finish this game well, then talk to them.
- **No background/subagent fan-out for lookahead research.** One game's research is a handful
  of sequential lookups you do inline.
- **When a game is done, stop and say so.** Report what you staged, then wait. Silence from the
  developer means they are playing, not that you should find more to do. Don't poll
  `queue_status` in a loop; check it once when you finish, and again when they speak.

A session where you enrich six games thoroughly, at their pace, beats one where you stage forty
and they trust none of it.

## Setup

1. Parse the developer's ask: which platform(s), which subset (their local collection, a
   search term, specific slugs, flagged entries), and the ROM directory if they named one.
2. **Start the curator if it isn't running.** Check for a UI socket
   (`$XDG_RUNTIME_DIR/missingno/ui-*.sock`); if the curator isn't up, launch it yourself in
   the background from the missingno repo root:

   ```
   cargo run --release -p missingno-curator -- [--rom-dir <dir>]
   ```

   **Release, always** — this is a standing exception to the repo's debug-by-default rule. The
   developer playtests through this window, and a debug-build emulator runs too slowly to judge
   a game by. Pass `--rom-dir` whenever the ask involves the developer's own collection — it
   auto-scans at startup. Wait for `ui-<pid>.sock` to appear (a cold release build takes several
   minutes), then connect with the **missingno-remote** MCP server's `attach` tool.
   If more than one UI socket is published, attach to the one whose app is
   `net.andyofniall.missingno-curator`.
3. Call `status` to see the queue counts, then build the work list:
   - "my collection" → `local_matches` (requires a scanned ROM dir).
   - a search term / platform → `search_games` with `backlog_only: true`.
   - flagged work → `list_flags`.
4. `queue_games` with the ordered keys. The first game auto-fetches (or uses the local dump)
   and starts playing immediately — the developer is now playtesting it.

## The loop (per game)

While the developer plays the current game:

1. `get_game` for the manifest and its open flags, and `find_duplicates` for merge
   candidates — always, for every game. A duplicate hit is a finding for your note (which
   entry should absorb which, and why); merging is the developer's call. Same-title hits
   that are genuinely different products (multicarts, unlicensed re-releases) are worth
   saying so explicitly, so the developer doesn't re-investigate next time.
2. Research the gaps — empty developer/description/license, suspicious titles, flag
   questions. Source preference: existing structured sources first (the gamedb itself,
   pouet.net data dumps, gbdev database, publisher/developer sites), then ordinary web
   pages. **Scraping etiquette is binding**: respect robots.txt, touch only documented or
   normal-user URLs, never probe guessed endpoints or parameters, one request at a time,
   and stop at the first anti-scraper signal. A blocked source is a fact to note, not an
   obstacle to work around.
3. `update_game` to stage what you found. Stage only facts you have a source for; leave
   unknown fields empty rather than guessing. Staged edits clear the curated stamp by
   design.
4. **Cover image** (`covers` in update_game — remote URLs only, we host nothing):
   - Commercial games: the curator's "Hasheous: cover & wiki" button (or WebFetch
     `hasheous.org/api/v1/Lookup/ByHash/sha1/<sha1>` yourself and use its
     `…/api/v1/images/<id>` URL) — Hasheous exists to serve frontends. Fallback:
     libretro-thumbnails (raw.githubusercontent.com/libretro-thumbnails/...), then a
     Wikipedia article's box art (upload.wikimedia.org — hotlinking is tolerated; note
     the image is usually fair-use, not free-licensed).
   - Homebrew: the project's own canonical host — its GitHub repo raw URLs (the gbdev
     pattern), or its documented page assets.
   - Demoscene: the prod's pouet.net page imagery.
   - Never store-CDN URLs (itch/steam image links churn); the store page belongs in
     `sources`, not `covers`.
5. **Wikipedia**: if the game has an article, stage it (`wikipedia` in update_game — it
   becomes the game's Wikipedia link). Hasheous often knows it for commercial games;
   otherwise a quick search — but only stage an article that is actually about this game,
   not its series or port.
6. **Hardware facts**: the curator auto-stages what a fetched/booted Game Boy header
   states (SGB/CGB enhancement, mapper) into the release — filling unknowns only, and
   reporting header-vs-db conflicts in the verify status, which you should surface in your
   note. Overrides via update_game when the truth differs from the header: `mapper`
   (GB/GBC — unlicensed carts lie) and `cart_type` (VCS — no headers, so the db drives the
   emulator's board choice; if a VCS playtest shows garbage, the board is the first
   suspect — check the game's known board on AtariAge or in Stella's properties and stage
   the correction, then have the developer replay).
7. `set_note` with a SHORT summary — three or four lines at most: what you staged, the
   single most load-bearing source, and anything to double-check. The developer has your
   chat window open beside the curator; full reasoning, source lists, and caveats belong
   in the conversation, not the note panel.
8. If a flag's answer is now established by the staged data and the developer has agreed
   (in conversation or by accepting the related edit), `resolve_flag`; otherwise propose the
   resolution in the note and leave the flag open.
9. Watch for the queue to advance: check `queue_status` when you finish a game's research;
   if the developer hasn't accepted yet, deepen the research or answer their questions —
   don't spin on polling, and don't stage anything for the next game to fill the time (see
   *One game at a time*). A quiet queue usually means they are still playing; talking to
   them about the game in front of them is the useful move.

The developer may simply close the curator window: the background process exiting with
status 0 means the session is over — summarize what was done and stop. Do not restart the
curator or treat the exit as a failure unless it actually reported one.

When the queue empties, summarize the session (games enriched, sources used, flags proposed/
resolved, anything skipped and why) and remind the developer that the staged work commits
from the curator's Commit button.

## Honesty rules

- Never fabricate a developer, date, description, or license. "Not found" is a valid result.
- Distinguish primary sources (the game's own page, its author) from aggregator claims, and
  say which kind each fact came from.
- If two sources disagree, stage nothing and put the conflict in the note.
