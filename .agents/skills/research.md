# Research

Research a specific question and document findings in the persistent knowledge base.

## Scope discipline

**You are a fact-finder, not a problem-solver.** Your report must follow the research report format defined in the skill invocation protocol in AGENTS.md. If you catch yourself writing interpretation, analysis, or recommendations — stop, delete it, and return to reporting facts.

The caller sent you a Question and Context. Answer the Question. Do not reason about the caller's problem, propose fixes, or expand scope. If you discover something tangential, note it as a one-liner in the "See also" section of your report.

## Before you start

Check what's already documented in `receipts/research/` for the relevant subsystem. Read existing documents before searching externally — avoid re-researching what's already known.

## Research strategy

Focus on finding **authoritative answers to specific behavioral questions**. Before searching, write down the exact question you need answered (e.g. "at what dot does LY increment?" not "how does LY work?"). This keeps research targeted and prevents scope creep.

Work through sources in this order, stopping when you have a clear, specific answer:

1. **Project docs and existing research**: Check AGENTS.md, README, commit messages, and `receipts/research/` — they often document hardware edge cases already discovered.
2. **Platform technical docs**: Search for the definitive hardware reference for the target system (e.g. Pan Docs for Game Boy). These are the highest-quality sources — prefer them over everything else.
3. **Hardware test results and analysis**: Look for documents, blog posts, or wiki pages where someone has measured real hardware behavior with an oscilloscope or test ROM and reported the results. These are factual observations, not interpretations.
4. **Community resources**: Forums, wikis, and blog posts from the emulation development community often document obscure hardware quirks with specific timing values and edge cases.
5. **Test suite sources**: Read the assembly/source for relevant test ROMs to understand exactly what they measure. This tells you what behavior to produce, with specific cycle counts.
6. **Reference emulators (last resort)**: Only consult emulator source code when documentation is insufficient. Reading source through WebFetch is unreliable — the AI summarizer often misinterprets code. If you must read emulator source, clone the repo locally and read the actual files. Extract factual hardware behavior (timing values, edge cases, state transitions) — do not copy architectural patterns, data structures, or implementation strategies.

### Allowed and forbidden tools

**Allowed:** `Read`, `Glob`, `Grep`, `Bash` (for `curl` to fetch specific URLs, `git clone` to clone repos). These are the research skill's tools.

**Forbidden:** `WebSearch`, `WebFetch`, `Skill` (no invoking other skills). If you reach for any of these, you have left the research skill's methodology. If your allowed tools can't answer the question, report what you found with `Confidence: low` and return — do NOT escalate to forbidden tools.

### Quality over quantity

- **One good source beats five bad fetches.** If Pan Docs answers your question, stop there. Don't keep searching for confirmation through emulator source code.
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
- `sameboy-sprite-handling.md` — how SameBoy implements the same behavior, citing specific file and line numbers
- `mbc1-bank-wrapping.md` — how MBC1 handles bank number overflow, citing hardware manual and Gambatte source

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

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

**You MUST continue working after writing your report.** The research phase is over; now resume as the caller. Concretely:

1. Write your report (Findings / Sources / Confidence / See also).
2. Write the caller's interpretation of the findings to `summary.md`. The findings are now on disk in two places (the research doc and the summary) — conversation memory of the research is no longer needed.
3. Re-read the caller's skill file (e.g. `.agents/skills/investigate.md`) and the active investigation's `summary.md` to restore the caller's context from disk. Work from the file state, not from conversation memory.
4. **Immediately continue the caller's workflow** — proceed to the next step based on what `summary.md` says, not on what you remember.

Do NOT end your turn after the report. Do NOT wait for further input. The report is a return value, not a stopping point.
