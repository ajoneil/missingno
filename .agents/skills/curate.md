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
  `queue_status` in a loop — `wait_for_action` blocks until they Accept or Flag and tells
  you which game is now up; that is the wait.

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
   candidates — always, for every game.

   **A dump-flag title is never a game: fold it in without asking.** An entry titled
   `… [a]`, `[a2]`, `[h1]`, `[t1]`, `[b]`, `[o1]`, `[!]`, `[fixed]`, or with a
   cataloguer's descriptor standing in for a name (`[different speed and colors]`),
   is the base game's dump that an import promoted to its own entry. `merge_game` it
   into the real entry, then classify what arrived: a `[h]`/`[t]` dump becomes a mod
   via `mark_hack`, an `[a]`/`[!]` alternate joins the matching release with a
   `label_artifact`, and a `[b]`/`[o]` defective dump is labelled as such beside the
   good one. The db should hold one entry per game, with every dump reachable from it.

   **Ask only when genuinely unsure** — when the "duplicate" may be a different
   product (a multicart, a same-titled game on another platform, a hack of a
   *different* game wearing this game's name), or when merging would lose a name or
   author the other entry records and you cannot preserve it. Then put the question
   in the note and leave both entries alone. Same-title hits that are genuinely
   different products are worth saying so explicitly, so the developer doesn't
   re-investigate next time.
2. **Check every artifact hash, not just one.** An entry's `artifacts` are what the db
   claims this game *is*; nobody re-checks them once imported, so bad groupings persist
   silently. Look up each sha1 and compare the answer against what the entry claims.

   Hasheous is the lookup. Run it for the entry in front of you and no further:

   ```
   gamedb verify-hashes --key <tree>/<slug>
   ```

   which asks per hash, records each answer on the artifact as `verified` evidence, and
   reports what contradicts the entry. **Never sweep the whole database** — that is
   thousands of requests at someone else's API for entries nobody is curating, and the
   command refuses an unbounded run for exactly that reason. Verification is part of
   looking at an entry, not a batch job. (The raw endpoint, when you want one hash by
   hand: `curl -s -H 'accept: application/json'
   https://hasheous.org/api/v1/Lookup/ByHash/sha1/<sha1>`.)

   Evidence is recorded per artifact and carries *how* it was checked — a `Signature`
   match keeps the name the database returned, which is the thing that distinguishes an
   original from a hack that hashes perfectly well. A `Playtest` verification is the
   developer's observation, never yours: record one only when they say so, and never
   infer it from an entry being accepted.

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

   **A hack's second dump belongs to that hack, not to a second mod.** `mark_hack`
   always makes a new mod, so use it only for the first dump of a hack; for anything
   further use `attach_dump_to_mod`: a later build is `as_version: true` with a label
   ("8K", "v2"), while an alternate or defective dump of a build already recorded is
   `as_version: false` (labelled "alt [a]", "overdump"). It also reaches a dump sitting
   in the *wrong* mod, so a build mistakenly filed as its own hack folds back in and the
   emptied mod disappears. A bad dump of a hack is that hack's evidence and never a work
   of its own — the same rule as a bad dump of a game.

   **Name a mod only what it is actually called.** A documented title goes in verbatim
   ("Adventure SI"). When the hack has no known name, do not dress a dump-flag descriptor
   up as one — `[h New Graphics]` is a cataloguer's note, not a title. Use the
   `Unnamed …` form the curator itself defaults to, so a reader can always tell a real
   title from one we supplied:

   - `Unnamed graphics hack of Adventure`
   - `Unnamed trainer of Adventure`
   - `Unnamed hack of <game>` when even the nature of the change is unclear

   Rename it with `update_mod` the moment a real name turns up. The same call records a
   mod's `author`, `date` and `url` — the signature entry usually names the group and
   year (Channel2, 2003), and those are worth staging when it does.

   **A title names the hack, not the game it was built from.** "Space Invaders Pinball"
   is a *Midnight Magic* hack; "Combat 500" is an *Indy 500* hack; a dump called
   "Galaga" turned out to be a River Raid hack. The signature entry states the base
   ("[h][River Raid]", "(MegaMania Hack)") — file the mod against *that* game, with
   `base_sha1` pointing at its dump. Filing by the title is how a false derivation gets
   recorded, and it reads perfectly plausibly afterwards.

   **Where a hack's dumps live is worth searching for.** Archive.org's documented
   search and metadata APIs (`advancedsearch.php`, `/metadata/<identifier>`) index the
   2600 hack scene with year and author in the item title, and expose per-file SHA-1 —
   which is how two same-named builds get told apart when the signature database knows
   neither. Prefer that over guessing which dump is which.
3. `verify_artifacts` early for entries with several dumps: confirmed originals gain
   recorded Signature evidence; DERIVED results ([h]/[t]/[tr]/[cr] — someone made these)
   are your cue to judge and `mark_hack` (find the mod's real name — the TOSEC bracket
   note is not it); DEFECTIVE results ([b]/[o] — a dumper's mistake, no author, never a
   mod) keep their evidence on the artifact: `label_artifact` them, and if the bad dump
   fabricated a release (an overdump fingerprinting as the wrong board), `move_artifact`
   it into the real release so the invented one is pruned. (Prototype)/(Beta) signature
   names suggest `split_release` — an editorial call: the build gets its own release with
   the right status, keeping a working title, never inheriting the retail date. "Unknown"
   is a normal result for homebrew and prototypes. Playtest verification is different:
   only the developer's "✓ works" button records it — never claim a dump was playtested.

   **A release holding many dumps is usually many releases.** The recurring import
   defect on VCS is one release carrying every dump anyone ever made of a game, stamped
   with whichever publisher happened to sort first — so the game's own original ends up
   filed under a regional reissue's publisher and year (River Raid and Pac-Man both had
   the Activision original inside a "Digitel 1983" release). The signature name states
   the publisher per dump: `split_release` each into its own release and set publisher,
   date and region from what the dump actually says. Dumps the signature database does
   not know go in a release with **no publisher**, labelled `unidentified` — leaving
   them where they sat asserts that whoever published that release published them too,
   which nothing supports.
4. Research the gaps — empty developer/description/license, suspicious titles, flag
   questions. Source preference: existing structured sources first (the gamedb itself,
   pouet.net data dumps, gbdev database, publisher/developer sites), then ordinary web
   pages. **The per-tree source hierarchy — which catalogues answer which fields for
   gb/gbc/vcs, and which are blocked — lives in `missingno-gamedb/SOURCES.md`. Read
   the section for the tree you are curating before searching the open web**, and add
   to it when a source proves itself: that file is the durable home for per-system
   cataloguing knowledge, not this skill. **Scraping etiquette is binding**: respect robots.txt, touch only documented or
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
   When a title correction leaves the slug wrong too (a typo'd import, a game that turned
   out to be something else), `rename_game` changes the slug — it moves the entry on disk
   and re-points flags and the queue, and its reply names the new `tree/slug` key: use
   that key for every later call. Fixing a title does not require renaming; do it when
   the slug would mislead someone browsing the tree.

   When `find_duplicates` (or a rename that collides) turns up an entry cataloguing the
   *same game* — an unlicensed reissue, a regional retitling — `merge_game` folds one
   into the other: the absorbed entry's releases and mods become the survivor's, its
   directory goes, and flags follow the surviving key. Dumps the target already holds
   are dropped rather than duplicated. **The merge is the developer's call, never
   yours** — propose it in the note and wait. A shared title alone is not sameness: a
   multicart or an unrelated game with the same name stays its own entry.

   A merged-in reissue usually shipped under its own publisher, region and sometimes
   its own name — `update_release` carries `publisher`, `regions` (a closed vocabulary:
   Japan, Usa, Europe, World, Taiwan, Germany, France, China, Spain, Italy, Australia,
   UnitedKingdom, Korea, HongKong, Sweden, Netherlands, Canada, Brazil) and `title`,
   which is the name *that release* shipped under when it differs from the game's
   canonical title. Set them from what the evidence names, and note that a TOSEC-style
   dump flag ("[a]", "[!]") is never a release title.
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
   - **Describe the port in front of you, not the arcade game it came from.** A
     conversion's article is mostly about the coin-op, and its facts do not carry over:
     the 2600 Missile Command has one missile base where the arcade has three, and
     Pac-Man's maze, fruit and escape passages are all different. Check that a link
     points at the port's own article (Space Invaders and Pac-Man both had entries
     linked to the arcade game), and where sources disagree about a mechanic, leave the
     detail out rather than pick — an arcade fact in a port's description is wrong in a
     way that reads perfectly.
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
11. When you finish a game's research, call `wait_for_action`: it blocks (~50s per call)
   until the developer Accepts (± recommendation) or Flags, and names the game now up.
   On timeout, call it again or answer their questions — don't spin on `queue_status`, and don't stage anything for the next game to fill the time (see
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
