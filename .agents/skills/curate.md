# Curate

Drive an interactive game-database curation session: queue games in the curator app, research
and stage metadata while the developer playtests each one, explain your reasoning, and roll
through the queue as they accept.

## Scope discipline

**You are a metadata researcher and queue operator, not the decision-maker.** Every fact you
stage must carry a source the developer can check; the `curated` stamp is theirs to give (the
Accept button), never yours. Do not push or edit manifest files directly — all writes
go through the curator's tools so the developer sees them live and the app owns the working
tree; the `commit` tool runs only on the developer's say-so.

**One game at a time — never work ahead of the playtest.** The only game you may write to is
the one `queue_status` reports as `current`. Curator writes land live in the developer's
window: staging an edit re-opens that entry for review and a note swaps what the editor
panel shows, so touching a queued-but-not-current game yanks the UI out from under someone
who is mid-playtest. Before every `update_game` or `resolve_flag`, confirm the
key you are about to pass equals the current game. Do not re-queue, `select_game`, or
`play_game` to move the queue along either: advancing is the developer's Accept, never yours.
When a side-edit to a non-current entry is genuinely required (the developer asked; a
sweep touches accepted entries), the edit re-selects that entry in their window —
**immediately `select_game` back to the current queue game** so they are never left
staring at the wrong entry mid-playtest.

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
   cargo run --release -p missingno-curator -- [--rom-dir <inbox>] [--collection-dir <collection>]
   ```

   `--rom-dir` is the **inbox**: the folder of to-be-curated ROMs that defines the
   session's work. `--collection-dir` is the **collection** of already-curated ROMs
   (`<collection>/<tree>/<slug>/`). The scan handles the boundary itself: an inbox file
   whose hash the collection already holds moves to `<inbox>/duplicates/` (a safety net —
   the developer empties it, nothing is deleted automatically), and **Accept moves the
   entry's inbox ROMs into the collection** — no manual archiving step. Entry rows mark
   inbox dumps `NEW` with a `Play new ▶` button, distinct from collection dumps' `Play ▶`.

   **Release, always** — this is a standing exception to the repo's debug-by-default rule. The
   developer playtests through this window, and a debug-build emulator runs too slowly to judge
   a game by. This applies to *every* launch, including **rebuild-and-restart**: when you change
   curator code and relaunch to apply it, build and run release (`cargo build --release` / the
   `target/release` binary) — never restart into a debug binary. Pass `--rom-dir` whenever the
   ask involves the developer's own collection — it auto-scans at startup. Wait for
   `ui-<pid>.sock` to appear (a cold release build takes several minutes), then connect with the
   **missingno-remote** MCP server's `attach` tool.
   If more than one UI socket is published, attach to the one whose app is
   `net.andyofniall.missingno-curator`.
3. Call `status` to see the queue counts, then build the work list:
   - "my collection" → `local_matches` (requires a scanned ROM dir).
   - a search term / platform → `search_games` with `backlog_only: true`.
   - flagged work → `list_flags`.
   - unmatched local ROMs → the `◆` "new (unmatched ROMs)" records the scan surfaced
     for local files that match no manifest; work them in the same alphabetical range you
     cover (see *Homebrew, alt-dumps, and unmatched records* below).
4. **Resolve a named ROM to its entry by hash, never by title.** When the ask points at
   files ("the 4-in-1 rom we have", "we missed X"), `sha1sum` the file and find the entry
   holding that artifact (grep the manifests) — a dump's entry is routinely filed under a
   different name than its filename (a TOSEC multicart extract, a reissue's title), and
   title-matching queues the wrong entry while the actual ask sits elsewhere. The same
   applies to range sweeps: a local file whose entry sorts outside the range still belongs
   to the batch.
5. `queue_games` with the ordered keys. The first game auto-fetches (or uses the local dump)
   and starts playing immediately — the developer is now playtesting it.

   **`queue_games` replaces the whole queue and starts its first key — it never appends.**
   Topping up mid-session therefore yanks the current playtest unless the call names the
   current game first: re-send the current key at the head, then the not-yet-played
   remainder, then the extension. Extend while the developer plays the game you just
   finished researching, never between your report and their Accept.

## The loop (per game)

While the developer plays the current game:

1. `get_game` for the manifest and its open flags, and `find_duplicates` for merge
   candidates — always, for every game.

   **The unit of work is the game, not the entry — hunt its other names.**
   `find_duplicates` only catches same-title entries; regional retitlings and re-skins
   hide under names it cannot see. Harvest alternate titles from every signal you already
   have — `[aka …]` in signature names, the catalogue page's alternate-title list
   (Atarimania lists them per game), where a Wikipedia search lands (a redirect to a
   differently-titled article is a same-game claim worth checking, not noise) — and
   `search_games` each name. Every hit is handled NOW, in this game's pass — verify the
   same-game claim against the catalogue and merge (or record why not) before moving on;
   "that entry comes later alphabetically" is never a reason to defer, because later
   batches won't know what this one learned.
   Done right, one entry ends up holding every skin as titled releases (the same game
   shipped as "Open Sesame", "I Want My Mommy", "Abre-te, Sesamo!" and "Apples and
   Dolls"); done lazily, four half-entries each get a shallow pass.

   **A dump-flag title is never a game: fold it in without asking.** An entry titled
   `… [a]`, `[a2]`, `[h1]`, `[t1]`, `[b]`, `[o1]`, `[!]`, `[fixed]`, or with a
   cataloguer's descriptor standing in for a name (`[different speed and colors]`),
   is the base game's dump that an import promoted to its own entry. `merge_game` it
   into the real entry, then classify what arrived: a `[h]`/`[t]` dump becomes a mod
   via `mark_mod`, an `[a]`/`[!]` alternate joins the matching release with a
   `label_artifact`, and a `[b]`/`[o]` defective dump is labelled as such beside the
   good one. The db should hold one entry per game, with every dump reachable from it.

   **Ask only when genuinely unsure** — when the "duplicate" may be a different
   product (a multicart, a same-titled game on another platform, a hack of a
   *different* game wearing this game's name), or when merging would lose a name or
   author the other entry records and you cannot preserve it. Then put the question
   to the developer in chat and leave both entries alone. Same-title hits that are genuinely
   different products are worth saying so explicitly, so the developer doesn't
   re-investigate next time.
2. **Check every artifact hash, not just one.** An entry's `artifacts` are what the db
   claims this game *is*; nobody re-checks them once imported, so bad groupings persist
   silently. Look up each sha1 and compare the answer against what the entry claims.

   Hasheous is the lookup. Run it for the entry in front of you and no further:

   ```
   gamedb verify-hashes --key <tree>/<slug>
   ```

   which asks per hash and reports what contradicts the entry. The answer is a
   session-time check reported in chat — **nothing is written into the manifest**; the
   dump's identity is re-checkable at any time from its hash. **Never sweep the whole database** — that is
   thousands of requests at someone else's API for entries nobody is curating, and the
   command refuses an unbounded run for exactly that reason. Verification is part of
   looking at an entry, not a batch job. (The raw endpoint, when you want one hash by
   hand: `curl -s -H 'accept: application/json'
   https://hasheous.org/api/v1/Lookup/ByHash/sha1/<sha1>`.)

   The signature name the database returns is what distinguishes an original from a
   hack that hashes perfectly well — read it carefully; it is the session's evidence
   even though it is not persisted.

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

   When a hash is confirmed to be a hacked or modified dump, `mark_mod` moves it out of
   the real dumps — and this is policy, not a question to put to the developer: a fan
   NTSC/PAL conversion imported as a *release* (the converter's name standing as
   publisher) gets `mark_mod`ed as Compatibility without asking, since the converter
   is an author, not a publisher. everything short of a total conversion — QoL, content changes, fan
   translations (the same game, as official localizations are) — becomes a mod attached to
   that game; supply its real name and homepage `url` when known. Only TotalConversion
   splits into its own entry. Never leave a hack lumped in
   with real dumps, and never silently delete the hash. When one release holds several
   legitimate dumps, `label_artifact` gives each a short distinguishing label. Mods are
   curated independently of their game — note anything you learn about a mod's quality,
   but the endorsement buttons are the developer's.

   **A hack's second dump belongs to that hack, not to a second mod.** `mark_mod`
   always makes a new mod, so use it only for the first dump of a hack; for anything
   further use `attach_dump_to_mod`: a later build is `as_version: true` with a label
   ("8K", "v2"), while an alternate or defective dump of a build already recorded is
   `as_version: false` (labelled "alt [a]", "overdump"). It also reaches a dump sitting
   in the *wrong* mod, so a build mistakenly filed as its own hack folds back in and the
   emptied mod disappears. A bad dump of a hack is that hack's evidence and never a work
   of its own — the same rule as a bad dump of a game.

   **Field altitude: release fields state the product, artifact labels state the dump.**
   A release's `title` is the name the product shipped under ("Choplifter" for a pirate
   cart sold under that name; "Wüstenschlacht"), its `label` a real edition descriptor
   ("Screen Search Console", "Light Green label"). TOSEC quality flags and dump
   commentary — `[p]`, `[o]`, "bad", "alt", "overdump" — always go on the artifact via
   `label_artifact`, never into release title/label, where they read as if the cart
   shipped wearing them.

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
   neither. Prefer that over guessing which dump is which — but **research only: no
   archive.org URLs go into `links`** (legality undecided; SOURCES.md carries the
   standing rule).
3. `verify_artifacts` early for entries with several dumps — the report lands in your
   session, not the manifest: DERIVED results ([h]/[t]/[tr]/[cr] — someone made these)
   are your cue to judge and `mark_mod` (find the mod's real name — the TOSEC bracket
   note is not it); DEFECTIVE results ([b]/[o] — a dumper's mistake, no author, never a
   mod) get `label_artifact`, and if the bad dump fabricated a release (an overdump
   fingerprinting as the wrong board), `move_artifact` it into the real release so the
   invented one is pruned. (Prototype)/(Beta) signature
   names suggest `split_release` — an editorial call: the build gets its own release with
   the right status, keeping a working title, never inheriting the retail date. "Unknown"
   is a normal result for homebrew and prototypes. Never claim a dump was playtested —
   playtesting is the developer's activity, and their Accept is its only record.

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
   normal-user URLs, one request at a time, and stop at the first anti-scraper signal.
   **Never fetch a URL you constructed** — not an endpoint, not a page slug, not a
   listing or index page. Every URL comes from a search result, a link read off a page
   you already fetched, or a filename convention SOURCES.md explicitly documents as
   constructible; if you are typing a path from what a site's URLs "usually look like",
   that is a guess — search instead. A blocked source is a fact to note, not an
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
   the slug would mislead someone browsing the tree. **Slug shape**: natural word order —
   a leading article stays and leads (`the-challenge-of-nexar`, matching the tree's
   `the-activision-decathlon`), never a cataloguer's sort-suffix
   (`challenge-of-nexar-the`) and never dropped entirely; strip TV-standard/board
   suffixes (`-ntsc`, `-pal-4k`) once the entry is the whole game rather than one dump
   of it; collapse apostrophe shrapnel (`custers-revenge`, never `custer-s-revenge` —
   the import splits possessives into a stray `-s-`). Fix a nonconforming slug as part
   of curating its entry, and remember the collection folder tracks the slug: after a
   rename of an accepted entry, move `<collection>/<tree>/<old-slug>/` to match.

   When `find_duplicates` (or a rename that collides) turns up an entry cataloguing the
   *same game* — an unlicensed reissue, a regional retitling — `merge_game` folds one
   into the other: the absorbed entry's releases and mods become the survivor's, its
   directory goes, and flags follow the surviving key. The absorbed entry's *title*
   is not carried anywhere — immediately after a merge, stamp it as `title` on each
   carried release that lacks one (`update_release`), or the name the reissue shipped
   under vanishes from the entry. Dumps the target already holds
   are dropped rather than duplicated. **The merge is the developer's call, never
   yours** — propose it in chat and wait. A shared title alone is not sameness: a
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
   - **Read a source before you write a word — order of operations, not a nicety.**
     Fetch and actually *read* the game's Wikipedia article, its manual, or a prototype
     writeup *first*; composing the description is the step that comes *after* reading it,
     never before. Writing from memory while intending to "attach a link later" is the
     exact failure: the link ends up decorating a description it never sourced, and a
     from-memory gameplay claim reads identically to a real one. If you have not read a
     source this turn, you have nothing to write yet — so go read one before writing, don't
     settle for empty (see the exhaust-sources rule below). A *scanned* manual is still a
     readable source: WebFetch can't extract its text, but downloading the PDF and opening it
     with the Read tool renders the pages as images you can read directly, turning a manual
     you'd otherwise have called unreadable into a primary source.
   - **Write the facts in your own words; never copy the prose.** The database is CC0;
     Wikipedia is CC BY-SA, and the two do not compose. Pasting or lightly reshuffling a
     lead paragraph would silently make the repo's LICENSE untrue for that entry, and
     nothing in the RON records where the text came from. Facts are free to take;
     expression is not.
   - **Gameplay only.** Say what the game is and what the player does. Never restate
     year, developer, publisher, platform, or **controller** — those are all structured
     fields (`controllers` included) the UI renders already, so repeating them ("one
     joystick", "uses the paddle") is pure duplication. Describe the *action* a control
     performs when it is the distinctive mechanic (you dial a knob to aim), not the device
     itself. (This is also why a Wikipedia lead is the wrong shape to lift: its first
     sentence is nothing but identity facts.)
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
     `Community` for AtariAge/store pages, `Source` for a creator's page, `DownloadPage`/
     `Download` for freeware ROM links — see *Homebrew* below — `TechnicalReference`,
     `Guide`, …). No link, no description: an unsourceable description is an assertion
     nobody can check later.
   - Two or three sentences. **Do not settle for an empty description — exhaust the
     sources first.** An empty field means you stopped researching too early, not that the
     game is undescribable. Work down: the port's own Wikipedia article → the per-tree
     catalogue in SOURCES.md (Atarimania for VCS, found via its search, never a guessed id)
     → the game's **manual**, read as page-images with the Read tool (works even when it is
     a scan). For obscure and unlicensed carts with no Compendium scan, **AtariAge often
     holds the manual as an HTML page** (the "HTML Manual" link off its software page) —
     the site is Cloudflare-blocked but reads fine through the Wayback Machine, and a
     Zellers instruction sheet recovered that way beats an empty field. Only after all of
     those genuinely come up empty is "not found" a result — and then say so to the
     developer and leave it flagged, rather than quietly blank.
6. **Cover image** (`covers` in update_game — remote URLs only, we host nothing):
   - **The curator auto-stages a Hasheous cover (and Wikipedia link) when a game loads** —
     a good default, but Hasheous groups variants under one record and its wiki mapping
     can be a different game entirely. So the first move is to VERIFY what's already
     staged: download the staged cover and *look* at it (the Read tool renders images),
     and open the staged article. Only go searching for other art when the staged image
     is wrong or missing — and when it is wrong, say so and clear or replace it.
   - Commercial games: the auto-staged Hasheous image (or `curl` the
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
   otherwise **find it by searching Wikipedia's opensearch API**
   (`https://en.wikipedia.org/w/api.php?action=opensearch&search=<title>&format=json`) and
   read the returned titles — never guess a `Foo_(video_game)` URL and treat its 404 as
   proof no article exists. A 404 on a guessed title says only that the guess was wrong; the
   real article (or the confirmation that there is none) comes from the search. Only stage an
   article actually about *this* game, not its series, a same-named TV show, or the arcade
   original it was ported from.
8. **Hardware facts**: the curator auto-stages what a fetched/booted Game Boy header
   states (SGB/CGB enhancement, mapper) into the release — filling unknowns only, and
   reporting header-vs-db conflicts in the verify status, which you should surface in your
   note. Overrides via update_game when the truth differs from the header: `mapper`
   (GB/GBC — unlicensed carts lie) and `cart_type` (VCS — no headers, so the db drives the
   emulator's board choice; if a VCS playtest shows garbage, the board is the first
   suspect — check the game's known board on AtariAge or in Stella's properties and stage
   the correction, then have the developer replay). **Controllers stage only on deviation
   from the platform default** (VCS: joystick) — Paddle, Driving, Keypad and friends get
   staged because the emulator must plug in the right device; a plain joystick game
   stages nothing, and an Accept vouches the default. The other staging case is sibling
   contrast: when one game's releases differ (a joystick conversion beside the paddle
   original), stage both sides so the difference is visible. **When the required
   controller is one the emulator doesn't support (VCS: Keypad, KidVid, MindLink),
   `raise_flag` an EmulationIncompatibility stating the requirement** — staging the
   controller alone does not surface the gap; the flag is what the emulator work
   queues on.
9. **The pre-report checklist — a game is not finished until every row has evidence
   from THIS pass.** Before reporting, re-read the entry (`get_game`) and confirm you
   can cite the tool call that satisfied each row:
   - **description** — a named source you read this pass, every clause traceable;
   - **covers** — the staged image downloaded and looked at, or an explicit absence
     reason (fan mock cleared, none exists);
   - **wikipedia** — a staged article verified to be about *this* game, or the
     opensearch call that returned empty. Search the PLAIN title (qualifiers in the
     query text — "Codebreaker Atari" — silently miss parenthetical titles like
     "Codebreaker (video game)"), and read every returned candidate. "No article
     exists" may only ever be said with that empty search on record — an unsearched
     absence claim is fabrication;
   - **manual** — the Compendium index consulted (and AtariAge-via-Wayback for
     unlicensed carts), linked or absent-with-reason;
   - **flags** — unsupported-controller and playtest-oddity checks done.
   The checklist exists because "I researched a lot" reliably masquerades as "I checked
   everything": the misses (Carnival's cover, Cosmic Ark's manual, a dozen unsearched
   wiki claims) were all fields where research petered out without a recorded answer.
10. **Report in chat** — there is no notes panel, deliberately. Say what you staged, the
   single most load-bearing source, and anything to double-check, in the conversation
   where the developer is already talking to you. Anything that must survive the session
   goes in a manifest field or a flag, never in prose.
11. If a flag's answer is now established by the staged data and the developer has agreed
   (in conversation or by accepting the related edit), `resolve_flag`; otherwise propose the
   resolution in chat and leave the flag open.
12. When you finish a game's research, call `wait_for_action`: it blocks (~50s per call)
   until the developer Accepts (± recommendation) or Flags, and names the game now up.
   On timeout, call it again or answer their questions — don't spin on `queue_status`, and don't stage anything for the next game to fill the time (see
   *One game at a time*). A quiet queue usually means they are still playing; talking to
   them about the game in front of them is the useful move.

The developer may simply close the curator window: the background process exiting with
status 0 means the session is over — summarize what was done and stop. Do not restart the
curator or treat the exit as a failure unless it actually reported one.

When the queue empties, summarize the session (games enriched, sources used, flags proposed/
resolved, anything skipped and why). Staged work commits through the `commit` tool — **only
when the developer asks**, normally at the end of a session, with a real commit message
describing the batch.

## Homebrew, alt-dumps, and unmatched records

The curator surfaces local ROMs that match no manifest as `◆` "new" records (the
"new (unmatched ROMs)" filter). They're where homebrew, prototypes, alt dumps and fan
re-dumps hide. **Sort each into one of these — don't reflexively make a new game:**

- **A new homebrew game** → curate it. Consolidate a multi-build homebrew (many version/
  region dumps of one game) into a *single* entry: `merge_game` the builds together, split
  into NTSC/PAL/PAL60 releases, `label_artifact` each dump by version (`v1.3`, `v0.1 (beta)`,
  `RetroN 77 edition`). `rename_game` the slug to the clean title afterward.
- **An alt dump / re-dump of a game already curated** (Atari Anthology extracts, `(Alt)`
  dumps) → `merge_game` the `◆` record into the existing entry, `move_artifact` the dump into
  the right release, `label_artifact` it (`Atari Anthology`, `alt [a2]`). No new entry.
- **An official enhanced re-release** (e.g. a modern Atari 2600+ "Enhanced Edition") → a
  distinct *release* of the original, **not** a fan mod — it's an official product.
- **A fan re-dump that won't load** (odd size; `failed to construct console from media`) →
  `split_release` it into its own release so it isn't mislabelled with the retail board
  (clear `cart_type` to auto-detect with an empty string), then `raise_flag`. Keep the dump;
  the flag records it's unplayable.

**Sourcing and licensing homebrew:**
- Most homebrew has **no Wikipedia article** — source the description from the creator's page
  or the AtariAge store *product* page (read it), never memory.
- A creator-page link must be for **this** version: a demake's page is not the original's
  page on another platform. If you could not open the page (cert error, 403), you have not
  verified it — do not link it.
- `license` is `Freeware` **only** when the creator released a free ROM and you can cite it
  (a "binaries for free" page, a free-download announcement). A **paid aftermarket cart with
  no free ROM** gets `license` left blank — treat it like a commercial game.

**AtariAge is link-only.** Forum threads and store are behind Cloudflare; a headless fetch
gets a 403 challenge, so no ROM is ever pulled from there — but store the AtariAge **release
thread** as the creator link regardless. To *read* a Cloudflare-blocked or dead page, fetch it
through the **Wayback Machine** (`archive.org/wayback/available?url=…`), which is fetchable.

**Freeware download links** (two roles, keep them separate): a `DownloadPage` is a page to
obtain the ROM (an AtariAge thread with the attachment, a "download here" page — human-
followed, often not fetchable); a `Download` is a direct fetchable file URL (a `.zip`/`.bin`).
Some hosts offer only a page (AtariAge); others also a direct file (a creator's
`wp-content/uploads` zip is usually fetchable even when the site's HTML pages 403). Verify a
`Download` by fetching it and hash-matching the dump — that also reveals its region.

**Playtest observations are data, not verdicts.** When the developer notes an oddity ("ominous
music", "flashes a lot"), first check whether it's the game being itself (the 2600 Asteroids
heartbeat and Video Cube's flicker are retail-normal — a 30-second search settles it) before
treating it as a hack or a bug. If it *is* abnormal, `raise_flag` — **facts only** (the
observed behaviour), never a cause hypothesis, repro plan, or "candidate for /investigate".

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
- If two sources disagree, stage nothing and put the conflict to the developer in chat.
