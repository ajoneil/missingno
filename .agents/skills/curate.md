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
2. **Check every artifact hash, not just one.** An entry's `artifacts` are what the db
   claims this game *is*; nobody re-checks them once imported, so bad groupings persist
   silently. Look up each sha1 and compare the answer against what the entry claims.

   Hasheous is the lookup — a documented API built for frontends to call, one request at
   a time:

   ```
   curl -s -H 'accept: application/json' \
     https://hasheous.org/api/v1/Lookup/ByHash/sha1/<sha1>
   ```

   Its `signature.rom.name` is a TOSEC/No-Intro-style filename, and the bracket flags are
   the signal: `[h]` hack, `[t]` trained, `[tr]` translation, `[cr]` cracked, `[a]`
   alternate, `[b]` bad dump, `[o]` overdump, `[f]` fixed. `signature.game` carries the
   year, publisher and video standard.

   What you are looking for:
   - **A derived work filed as an original.** A `[h]`/`[t]`/`[tr]` dump sitting in a
     release that claims the original publisher and year is a defect, not a variant — the
     hack is its own work and wants its own entry with `mod_of` pointing at the base
     sha1. Report it; never quietly drop an artifact.
   - **Artifacts that disagree with the release** — a PAL dump inside an NTSC release, a
     different publisher, a year that is not close.
   - **A 404** ("hash not found in the signature database") is a fact worth recording,
     not a failure. It usually means a homebrew, prototype or private dump. Say which
     hashes were unknown rather than implying the whole set checked out.

   Two cautions. Signature databases disagree with encyclopaedias about dates — TOSEC
   years in particular are often a year or two off a documented release date; a
   disagreement is a conflict to report, never a licence to restage the date. And the
   curator's "Hasheous: cover & wiki" button hashes *the artifact being played*, so if the
   playtest booted a hack, that button will fetch the hack's metadata: check which dump
   the entry actually leads with before trusting what it returns.

   When a hash is confirmed to be a hacked or modified dump, `mark_hack` moves it out of
   the real dumps: everything short of a total conversion — QoL, content changes, fan
   translations (the same game, as official localizations are) — becomes a mod attached to
   that game; supply its real name and homepage `url` when known. Only TotalConversion
   splits into its own entry. Never leave a hack lumped in
   with real dumps, and never silently delete the hash. When one release holds several
   legitimate dumps, `label_artifact` gives each a short distinguishing label. Mods are
   curated independently of their game — note anything you learn about a mod's quality,
   but the endorsement buttons are the developer's.
3. Research the gaps — empty developer/description/license, suspicious titles, flag
   questions. Source preference: existing structured sources first (the gamedb itself,
   pouet.net data dumps, gbdev database, publisher/developer sites), then ordinary web
   pages. **Scraping etiquette is binding**: respect robots.txt, touch only documented or
   normal-user URLs, never probe guessed endpoints or parameters, one request at a time,
   and stop at the first anti-scraper signal. A blocked source is a fact to note, not an
   obstacle to work around. **But confirm the block is the site's, not your tool's**: a 403
   from WebFetch often means its user-agent was refused, and the same documented endpoint
   answers a plain `curl` fine. Retrying a documented API with a normal client is not
   working around a block — abandoning a source that would have answered is a worse
   failure than one extra request. Declaring a source blocked is a claim; check it.
4. `update_game` to stage what you found. Stage only facts you have a source for; leave
   unknown fields empty rather than guessing. Staged edits clear the curated stamp by
   design. **Record each source as a link in the same call** (`links`:
   `{name, url, link_type}` — upserts by name): links live in the manifest and survive;
   notes do not. A description sourced from an AtariAge thread means a link to that thread.
5. **Descriptions** — the field most likely to accumulate quiet fiction, so it has its own
   rules:
   - **Write the facts in your own words; never copy the prose.** The database is CC0;
     Wikipedia is CC BY-SA, and the two do not compose. Pasting or lightly reshuffling a
     lead paragraph would silently make the repo's LICENSE untrue for that entry, and
     nothing in the RON records where the text came from. Facts are free to take;
     expression is not.
   - **Gameplay only.** Say what the game is and what the player does. Never restate
     year, developer, publisher or platform — those are structured fields the UI renders
     already, so repeating them is pure duplication. (This is exactly why a Wikipedia lead
     is the wrong shape to lift: its first sentence is nothing but identity facts.)
   - **Every clause must be traceable to a named source.** Not "mostly sourced with a bit
     of common knowledge" — the failure mode is a true-sounding fact you actually supplied
     from memory, which reads identically to a sourced one and survives every review. If
     you cannot point at where a clause came from, cut the clause.
   - **Stage a link that backs it.** The source belongs in `links` (`Wiki` for Wikipedia,
     `Community` for AtariAge/forum pages, `TechnicalReference`, `Guide`, …). No link, no
     description: an unsourceable description is an assertion nobody can check later.
   - **A note is not a receipt.** `set_note` writes to an in-memory map in the running app
     and is never persisted — it vanishes when the curator closes. Anything that must
     survive the session goes in a field, never in the note.
   - Two or three sentences. If a game is obscure enough that no source describes its
     gameplay, leave the field empty and say so — "not found" is a valid result.
6. **Cover image** (`covers` in update_game — remote URLs only, we host nothing):
   - Commercial games: the curator's "Hasheous: cover & wiki" button (or `curl` the
     lookup from step 2 yourself and use the `…/api/v1/images/<id>` URL from its
     `attributes`) — Hasheous exists to serve frontends. Take the image from the record
     of the dump you actually mean: a hack's record carries the hack's art. Fallback:
     libretro-thumbnails (raw.githubusercontent.com/libretro-thumbnails/...), then a
     Wikipedia article's box art (upload.wikimedia.org — hotlinking is tolerated; note
     the image is usually fair-use, not free-licensed).
   - Homebrew: the project's own canonical host — its GitHub repo raw URLs (the gbdev
     pattern), or its documented page assets.
   - Demoscene: the prod's pouet.net page imagery.
   - Never store-CDN URLs (itch/steam image links churn); the store page belongs in
     `sources`, not `covers`.
7. **Wikipedia**: if the game has an article, stage it (`wikipedia` in update_game — it
   becomes the game's Wikipedia link). Hasheous often knows it for commercial games;
   otherwise a quick search — but only stage an article that is actually about this game,
   not its series or port.
8. **Hardware facts**: the curator auto-stages what a fetched/booted Game Boy header
   states (SGB/CGB enhancement, mapper) into the release — filling unknowns only, and
   reporting header-vs-db conflicts in the verify status, which you should surface in your
   note. Overrides via update_game when the truth differs from the header: `mapper`
   (GB/GBC — unlicensed carts lie) and `cart_type` (VCS — no headers, so the db drives the
   emulator's board choice; if a VCS playtest shows garbage, the board is the first
   suspect — check the game's known board on AtariAge or in Stella's properties and stage
   the correction, then have the developer replay).
9. `set_note` with a SHORT summary — three or four lines at most: what you staged, the
   single most load-bearing source, and anything to double-check. The developer has your
   chat window open beside the curator; full reasoning, source lists, and caveats belong
   in the conversation, not the note panel. The note is a live talking point and nothing
   more — it lives in memory in the running app and is gone when the window closes, so
   never let it be the only record of something that matters.
10. If a flag's answer is now established by the staged data and the developer has agreed
   (in conversation or by accepting the related edit), `resolve_flag`; otherwise propose the
   resolution in the note and leave the flag open.
11. Watch for the queue to advance: check `queue_status` when you finish a game's research;
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
- **The dangerous claim is the one that feels obvious.** Outright invention is easy to avoid;
  what actually gets through is a true-sounding fact you supplied from your own knowledge
  while believing you read it. It reads exactly like a sourced fact, and being usually
  correct is what makes it survive review. Before staging, check each claim against what the
  source in front of you actually says — and when the check contradicts you, say so plainly
  rather than quietly correcting.
- Distinguish primary sources (the game's own page, its author) from aggregator claims, and
  say which kind each fact came from.
- If two sources disagree, stage nothing and put the conflict in the note.
