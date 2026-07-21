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

## Setup

1. Parse the developer's ask: which platform(s), which subset (their local collection, a
   search term, specific slugs, flagged entries), and the ROM directory if they named one.
2. **Start the curator if it isn't running.** Check for a UI socket
   (`$XDG_RUNTIME_DIR/missingno/ui-*.sock`); if the curator isn't up, launch it yourself in
   the background from the missingno repo root:

   ```
   cargo run -p missingno-curator -- [--rom-dir <dir>]
   ```

   (debug build; pass `--rom-dir` whenever the ask involves the developer's own collection —
   it auto-scans at startup). Wait for `ui-<pid>.sock` to appear (the build can take a couple
   of minutes cold), then connect with the **missingno-remote** MCP server's `attach` tool.
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

1. `get_game` for the manifest and its open flags.
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
4. `set_note` with your reasoning — what you changed, the source for each fact (URL or
   dataset name), your confidence, and anything the developer should double-check or decide
   (e.g. a flag you'd resolve and why). Keep it short enough to read during a playtest.
5. If a flag's answer is now established by the staged data and the developer has agreed
   (in conversation or by accepting the related edit), `resolve_flag`; otherwise propose the
   resolution in the note and leave the flag open.
6. Watch for the queue to advance: check `queue_status` when you finish a game's research;
   if the developer hasn't accepted yet, deepen the research or answer their questions —
   don't spin on polling.

When the queue empties, summarize the session (games enriched, sources used, flags proposed/
resolved, anything skipped and why) and remind the developer that the staged work commits
from the curator's Commit button.

## Honesty rules

- Never fabricate a developer, date, description, or license. "Not found" is a valid result.
- Distinguish primary sources (the game's own page, its author) from aggregator claims, and
  say which kind each fact came from.
- If two sources disagree, stage nothing and put the conflict in the note.
