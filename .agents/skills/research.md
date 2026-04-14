# Research

Research a specific question and document findings in the persistent knowledge base.

## Scope discipline

**You are a fact-finder, not a problem-solver.** Your report must follow the research report format defined in the skill invocation protocol in AGENTS.md. If you catch yourself writing interpretation, analysis, or recommendations — stop, delete it, and return to reporting facts.

The caller sent you a Question and Context. Answer the Question. Do not reason about the caller's problem, propose fixes, or expand scope. If you discover something tangential, note it as a one-liner in the "See also" section of your report.

## Before you start

Check what's already documented in `receipts/research/` for the relevant subsystem. Read existing documents before searching externally — avoid re-researching what's already known.

## Research strategy

Focus on finding **authoritative answers to specific behavioral questions**. Before searching, write down the exact question you need answered (e.g. "at what dot does LY increment?" not "how does LY work?"). This keeps research targeted and prevents scope creep.

**The goal is always to understand what the real hardware does.** When consulting any source — documentation, hardware tests, or emulator code — the question you're answering is "what does the hardware do?", never "how does emulator X implement this?".

Work through sources in this order, stopping when you have a clear, specific answer:

### Primary sources (ground truth)

These describe what the hardware actually does. They are the source of truth.

1. **Project docs and existing research**: Check AGENTS.md, README, commit messages, and `receipts/research/` — they often document hardware edge cases already discovered.
2. **Platform technical docs**: Search for the definitive hardware reference for the target system (e.g. Pan Docs for Game Boy). These are the highest-quality written sources.
3. **Hardware test results and analysis**: Documents, blog posts, or wiki pages where someone has measured real hardware behavior with an oscilloscope, logic analyzer, or test ROM and reported the raw results. These are direct observations of the hardware, not interpretations or models.
4. **Propagation delay analysis**: `receipts/resources/gb-propagation-delay-analysis/` provides static analysis of GateBoy's netlist identifying signal races and deep combinatorial paths that cause propagation delay on real hardware. Coverage extends beyond the PPU, though PPU races have the largest observable effects. Especially valuable for one-dot timing discrepancies. Key files in `receipts/resources/gb-propagation-delay-analysis/output/`:
   - `race_pairs_report.md` — Signal race pairs with observable effects (e.g. "OAM scan extends one dot", "tile fetch runs one extra cycle")
   - `critical_paths_report.md` — Overview of deepest combinatorial paths
   - `operational_paths.md` — Per-dot paths grouped by functional area
   - `signal_concordance.md` — Maps GateBoy 4-letter cell names to Pan Docs register names
   - `race_pairs.json` / `critical_paths.json` — Machine-readable data for specific signal lookups
   When researching PPU timing, check whether the behavior in question involves a known race pair or deep path. The race pairs report groups findings by observable effect, making it easy to match against emulator symptoms.
5. **Cross-emulator execution traces (gbtrace)**: The `receipts/resources/gbtrace/` provides tooling for capturing and comparing execution traces across emulators. Tracked emulators: gambatte, gateboy, docboy, missingno, sameboy. DocBoy provides T-cycle granularity traces. Reference traces for 15 test suites are hosted at [ajoneil.github.io/gbtrace](https://ajoneil.github.io/gbtrace/) with structured manifests.
   - **Capture**: `GBTRACE_PROFILE=gbmicrotest cargo test -p missingno-gb --features gbtrace -- <test_name>` → writes `receipts/traces/<rom>.gbtrace`.
   - **Inspect individual traces**: `gbtrace info <file>` shows summary; `gbtrace query <file> --where pc=0x0150` finds specific entries; `gbtrace query <file> --last 20` shows end-of-test state; `gbtrace frames <file>` shows frame boundaries. For larger tests, individual inspection is often more productive than diffing.
   - **Compare**: `gbtrace diff <missingno.gbtrace> <reference.gbtrace> --sync "lcdc&0x80" --exclude div,tac,if_` reports first divergence and per-field divergence counts. Best for small focused tests (gbmicrotest).
   - **Render**: `gbtrace render <file> -o <dir>` renders LCD frames from pixel trace data to PNG for visual comparison.
   - **Reference traces**: Fetch manifest with `curl -s https://ajoneil.github.io/gbtrace/tests/{suite}/manifest.json`, download traces at `tests/{suite}/{test}_{emulator}_{status}.gbtrace`.
   - Profiles are per-suite TOML files in `receipts/resources/gbtrace/test-suites/*/profile.toml` (e.g. `gbmicrotest`, `blargg`, `mooneye`).
6. **Hardware timing measurements** (`receipts/resources/gb-timing-data/`): Empirical cycle-level timing data from real Game Boy hardware. Contains parameterized measurement campaigns (TOML definitions in `campaigns/`) covering PPU mode timing, sprite penalties, OAM/VRAM lock boundaries, and timer subsystem timing. Results are CSV files with multi-dimensional sweep data. **Status: data collection in progress.** Check whether a relevant campaign exists for the behavior in question — if it has results, they are definitive hardware measurements. If a relevant campaign exists but has no data yet, note this in your report.
7. **Test suite sources**: Read the assembly/source for relevant test ROMs to understand exactly what they measure. The expected values in a test ROM that passes on hardware are hardware facts — they tell you what the hardware produces, with specific cycle counts and pixel values.
8. **Hardware test harness** (`receipts/resources/slowpeek/`): Programmable harness for running custom SM83 assembly on real Game Boy hardware with cycle-precise interrupt-driven measurements. **Status: functional against missingno emulator; hardware serial bridge in development.** When no existing data source answers the question, note that a Slowpeek test could provide the definitive measurement. Do not attempt hardware mode yet, but do flag when it would be valuable.

### Secondary sources (community knowledge)

Useful for filling gaps, but may reflect incomplete understanding or outdated models.

7. **Community resources**: Forums, wikis, and blog posts from the emulation development community. These often document obscure hardware quirks, but treat them as leads to verify against primary sources rather than as facts. Community consensus can be wrong — the history of emulation is full of "everyone implemented it this way" turning out to be inaccurate when someone finally tested on hardware.

### Tertiary sources (confirmation only)

**Emulator source code is a model of the hardware, not the hardware itself.** Another developer's reverse-engineered approximation is not a primary source, no matter how accurate the emulator is. Use it only to confirm or fill in details after you already have a primary-source understanding.

8. **Highly accurate emulator source (confirmation and gap-filling)**: Emulator source from projects with strong hardware accuracy (e.g. SameBoy, Gambatte) may be consulted **after** you have established the expected behavior from primary sources. Their role is to:
   - **Confirm** a timing value or edge case you already found in docs/tests/hardware measurements.
   - **Fill gaps** where primary sources are silent — but flag these findings as `Confidence: low` since you're trusting another developer's reverse engineering rather than a direct hardware observation.
   
   Clone the repo locally and read the actual files (WebFetch loses critical details). **Report what the emulator does, attributed to the emulator**, not presented as hardware fact. "SameBoy transitions to mode 0 at dot N (file:line)" — not "the hardware transitions to mode 0 at dot N". The caller can decide how much weight to give an emulator-sourced finding. Do not copy architectural patterns, data structures, or implementation strategies — these are the emulator author's design choices, not hardware behavior.
   
   **Never use emulator source as the starting point for understanding a behavior.** If you find yourself reading SameBoy to figure out how something works and then looking for docs to confirm it, you've inverted the hierarchy. The emulator's model shapes your mental model, and confirmation bias makes the docs seem to agree. Start from docs and hardware tests; go to emulator source only to check details.

### Allowed and forbidden tools

**Allowed:** `Read`, `Glob`, `Grep`, `Bash` (for `curl` to fetch specific URLs, `git clone` to clone repos). These are the research skill's tools.

**Forbidden:** `WebSearch`, `WebFetch`, `Skill` (no invoking other skills). If you reach for any of these, you have left the research skill's methodology. If your allowed tools can't answer the question, report what you found with `Confidence: low` and return — do NOT escalate to forbidden tools.

### Research finds facts, it does not derive them

**If answering the question requires reasoning through multiple steps of logic, arithmetic, or simulation — stop.** You are no longer researching; you are reasoning. Research finds stated facts in sources. Examples:

- **Research question**: "What does Pan Docs say the first pixel output dot is?" → Read Pan Docs, report the stated value. This is research.
- **NOT a research question**: "Given that the interrupt dispatch takes 20 cycles and the handler has 12 cycles of setup, at what scanline dot does the first BGP write land?" → This requires cycle-counting arithmetic. It belongs in `/instrument` (instrument the emulator and log the actual dot) or `/analyze` (interpret measurements).
- **Research question**: "What pixel position does the `m3_bgp_change` reference screenshot show for the first palette transition?" → Look at the image, report the pixel coordinate. This is research.
- **NOT a research question**: "Working backwards from the reference screenshot's pixel position and the test ROM's cycle timing, at what Mode 3 dot does BGP take effect?" → This requires multi-step derivation. Report the raw facts (the pixel position, the cycle counts from the source) and let the caller's `/analyze` step do the derivation.

The test: **can you answer by quoting or directly observing a source, or do you need to calculate?** If you need to calculate, report the raw inputs and return. The caller will invoke `/analyze` or `/instrument` to do the derivation.

This is especially important when reading assembly source code for test ROMs. You can report what the code does (instructions, register values, addresses written to) but you must not trace execution to derive timing values. "The handler writes to BGP via `ld [c], a`" is a fact you can report. "The BGP write happens at scanline dot 92" requires counting cycles through the interrupt dispatch, handler preamble, and instructions — that's derivation, not research.

### Quality over quantity

- **One good primary source beats five emulator readings.** If Pan Docs or a hardware test answers your question, stop there. Don't keep searching for confirmation through emulator source code — that inverts the hierarchy. The exception is when primary sources give a general description but you need a specific value (e.g. "sprites extend mode 3" but no exact dot count) — then emulator source can fill the gap, flagged as low confidence.
- **Don't use WebFetch for technical content.** The AI summarizer loses critical details like exact cycle counts, conditional branches, timing values, and subtle state machine transitions. Instead, use `Bash` with `curl` to fetch raw page content and read it yourself: `curl -s 'URL' | sed 's/<[^>]*>//g'` for HTML pages, or clone repos and use `Read` for source code. You can read the raw text directly — don't rely on an AI middleman to interpret technical documentation for you.
- **Stop when you have the answer.** Research is not exploration — you have a specific question. Once answered with a credible source, document it and move on.
- **If a source doesn't have what you need after one fetch, move to the next source.** Don't re-fetch the same URL with different prompts hoping for different results.

## Documenting findings

Write one document per significant finding, **immediately when you learn something** — not deferred to the end. The document on disk is the deliverable, not the conversation context. Once a finding is written to a file, the conversation memory of that finding is disposable — if you need to reference it later, re-read the file.

Each document should include:

- **Summary of the behavior**: what the hardware does, with enough detail that someone can implement it without re-reading the original source.
- **Citations**: URL, doc name, or emulator repo + file path + line numbers. Be specific.
- **Ambiguities or conflicts**: note where sources disagree or where behavior is undocumented.

If you consult two different sources in the same step and learn distinct things, that's two documents.

### Review before finishing

After writing a document, **re-read it in full** and edit out:

- **Thinking-out-loud**: "Wait, actually...", "Hmm, let me reconsider", "No...", "Actually looking more carefully...". These are internal reasoning — resolve them to a conclusion and state only the conclusion.
- **Contradictions**: If you corrected yourself mid-document, delete the wrong version and keep only the right one.
- **Hedging without resolution**: "This seems like..." or "I think..." — either confirm it with a source or flag it explicitly as uncertain. Don't leave vague impressions.
- **Unnecessary questions**: Rhetorical questions you then answer — just state the answer.

The document should read as a clean reference, not a transcript of your investigation process.

### Examples

- `mode3-sprite-penalty.md` — how sprites extend mode 3, citing Pan Docs and test ROM data
- `mbc1-bank-wrapping.md` — how MBC1 handles bank number overflow, citing hardware manual and test ROM analysis
- `ly-increment-timing.md` — exact dot on which LY increments, citing TCAGBD measurements and hardware oscilloscope data
- `write-conflict-mechanism.md` — hardware write conflict timing values extracted from accurate emulator source, reported as hardware facts

### Updating existing documents

If a document already exists for the topic, **update it** with the new information rather than creating a duplicate. Add new citations and note any changes to your understanding.

## Output location

There are two output locations depending on the nature of the finding:

### General hardware knowledge → `receipts/research/`

Findings about how the hardware works that would be useful to any future investigation go here:

```
receipts/research/systems/<platform>/<subsystem>/<topic>.md
```

Organised by platform and subsystem. For Game Boy:

```
receipts/research/
└── systems/
    └── game-boy/
        ├── cpu/
        ├── ppu/
        ├── apu/
        ├── timers/
        ├── interrupts/
        ├── memory/
        ├── dma/
        ├── serial/
        └── joypad/
```

File names should be descriptive kebab-case (e.g. `oam-scan-duration.md`, `daa-behavior.md`).

Create subdirectories as needed — the structure should match how you'd look something up, not how you happened to discover it.

### Investigation-specific findings → investigation's `research/` folder

Findings that are specific to a particular test ROM or investigation — such as test ROM source analysis, expected values for a specific test, or interpretation of diagnostic output — go in the active investigation's research folder:

```
receipts/investigations/<session>/research/<topic>.md
```

Use this location when:
- Analyzing a specific test ROM's source code or assembly
- Documenting expected values or behavior for a specific test
- Recording hypotheses or interpretations tied to a particular failure

Use the general location when:
- The finding describes how the hardware works (not how a test works)
- A future investigation into the same subsystem would benefit from the finding
- The information comes from platform documentation or hardware analysis

When called from an active investigation, check whether the research question is about hardware behavior (general) or test-specific analysis (investigation-specific), and write to the appropriate location.

## After research is complete

1. Write your report to the specified output path (Findings / Sources / Confidence / See also).
2. **Re-read the report in full** and edit out thinking-out-loud, contradictions, and hedging (see "Review before finishing" above).
3. **Do not update `summary.md`.** The caller owns summary.md and will incorporate the findings.
4. **Stop.** Your job is done. The caller reads the receipt file and decides what to do next.
