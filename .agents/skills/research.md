# Research

Research hardware behavior and document findings in the persistent knowledge base.

**This skill is a subroutine, not a stopping point.** After completing research, immediately return to the task that prompted it. Do not wait for further user input — apply the findings and continue working.

**IMPORTANT**: When this skill finishes (document written and reviewed), your VERY NEXT action must be applying the findings — editing code, running diagnostics, updating summary.md, etc. Do NOT end your turn after writing the research document. The research answer is useless until you act on it.

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

### Quality over quantity

- **One good source beats five bad fetches.** If Pan Docs answers your question, stop there. Don't keep searching for confirmation through emulator source code.
- **Don't use WebFetch for technical content.** The AI summarizer loses critical details like exact cycle counts, conditional branches, timing values, and subtle state machine transitions. Instead, use `Bash` with `curl` to fetch raw page content and read it yourself: `curl -s 'URL' | sed 's/<[^>]*>//g'` for HTML pages, or clone repos and use `Read` for source code. You can read the raw text directly — don't rely on an AI middleman to interpret technical documentation for you.
- **Stop when you have the answer.** Research is not exploration — you have a specific question. Once answered with a credible source, document it and move on.
- **If a source doesn't have what you need after one fetch, move to the next source.** Don't re-fetch the same URL with different prompts hoping for different results.

## Documenting findings

Write one document per significant finding, **immediately when you learn something** — not deferred to the end.

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
