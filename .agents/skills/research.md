# Research

Research hardware behavior and document findings in the persistent knowledge base.

## When to use

Any time you need to understand how real hardware behaves — during compatibility investigations, feature implementation, or standalone research tasks.

## Before you start

Check what's already documented in `receipts/research/` for the relevant subsystem. Read existing documents before searching externally — avoid re-researching what's already known.

## Research strategy

Work through sources in this order, stopping when you have enough confidence:

1. **Project docs and existing research**: Check AGENTS.md, README, commit messages, and `receipts/research/` — they often document hardware edge cases already discovered.
2. **Platform technical docs**: Search for the definitive hardware reference for the target system (e.g. Pan Docs for Game Boy).
3. **Test suite sources**: Find the assembly/source for relevant test ROMs to understand exactly what they measure.
4. **Reference emulators**: Search GitHub for accurate emulators of the target platform with permissive licenses (MIT, Apache, zlib, public domain). Use them to understand **what the hardware does**, not how to structure your code. Extract factual hardware behavior (timing values, edge cases, state transitions) — do not copy architectural patterns, data structures, or implementation strategies. Your emulator has its own architecture; the goal is to know what behavior to produce, not how another project chose to produce it. Prefer emulators known for accuracy over speed.
5. **Community resources**: Forums, wikis, and blog posts from the emulation development community often document obscure hardware quirks.

Cross-reference multiple sources to build confidence in what the correct behavior should be.

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
