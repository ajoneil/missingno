# Prepare Release

Cut the monthly calendar release (`vYY.M`): confirm everything is pushed, draft the changelog with the developer, tag, and prepare the Flathub manifest for them to submit.

## Scope discipline

**The developer owns the wording and the Flathub submission.** Draft the changelog, but nothing is committed, tagged or pushed until they have read it and agreed on the wording. A tag is public once pushed, and moving it later means a force-push.

**Never commit, amend, push, or open a PR in the Flathub repo.** Flathub does not allow AI automation there. Edit the files in the working tree and hand over.

**No version bump.** The workspace version (`[workspace.package] version` in the root `Cargo.toml`) is bumped when a dev cycle opens, so it already reads the release being cut. If it doesn't match, stop and raise it.

## Step 1 — Everything is pushed

Fetch, then confirm each of these is in sync with origin:

- missingno `main`, with a clean working tree.
- The `missingno-gamedb` submodule: `git submodule status` shows no `+`, and its `main` matches origin.
- morepork: the commit `Cargo.lock` pins for `morepork` is on morepork's origin `main`.

Old local experiment branches are not part of a release; ignore them. Report anything unpushed on the mains and ask before pushing it.

## Step 2 — Draft the changelog

The changelog is the `<releases>` block in `net.andyofniall.missingno.metainfo.xml`. Survey the changes since the previous tag:

```bash
git log --first-parent --format='%h %s' <prev-tag>..HEAD
git -C missingno-gamedb log --oneline $(git ls-tree <prev-tag> missingno-gamedb | awk '{print $3}')..HEAD
```

Read commit bodies where a subject is unclear. Draft 3–5 bullets in the style of earlier entries. They are terse, user-facing and noun-led, e.g. "ColecoVision support" or "Minor Super Game Boy fixes".

- List only what the shipped app surfaces. A capability that only the core or a library host uses is not a release note.
- Group fixes in one area into a single short bullet rather than describing each mechanism.

**Present the draft and the list of changes you left out, then stop.** Edit it with the developer until they confirm the final wording.

## Step 3 — Apply, commit, tag

After the wording is confirmed:

1. Add `<release version="YY.M" date="YYYY-MM-DD">` at the top of `<releases>`.
2. Repoint the screenshot URLs (`/v<prev>/screenshots/`) to the new tag. Re-shoot the images themselves only when asked.
3. If a system was added, update the system list in the metainfo `<description>` and in the README's opening line.
4. Run `appstreamcli validate --no-net net.andyofniall.missingno.metainfo.xml`.
5. Commit as `Release vYY.M`, make a lightweight tag `vYY.M`, and push `main` and the tag.

## Step 4 — Flathub manifest

The manifest lives in https://github.com/flathub/net.andyofniall.missingno. It is one checkout per machine, cloned over SSH (`git@github.com:flathub/net.andyofniall.missingno.git`); an HTTPS remote prompts for a password on push.

1. Update `master` and create the branch `vYY.M` from it.
2. In `net.andyofniall.missingno.json`, set the missingno git source's `tag` and `commit` to the release commit.
3. Use the newest runtime. `runtime-version` should be the newest `org.freedesktop.Platform` branch that `flatpak remote-ls flathub --runtime` lists. The matching `org.freedesktop.Sdk` and `org.freedesktop.Sdk.Extension.rust-stable` branches must exist too.
4. Regenerate `cargo-sources.json` from the release `Cargo.lock`. This and the later steps use Flathub's `org.flatpak.Builder` app (install it from `flathub` if missing). Its sandbox has a private `/tmp`, so give it paths outside `/tmp`:
   ```bash
   flatpak run --command=flatpak-cargo-generator org.flatpak.Builder "$PWD/Cargo.lock" -o <flathub-checkout>/cargo-sources.json
   ```
5. Lint the manifest: `flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest <flathub-checkout>/net.andyofniall.missingno.json`.

Leave the changes uncommitted.

## Step 5 — Test build

Build the manifest as Flathub would, in `receipts/flatpak-build/`. Don't build in a tmpfs `/tmp`, which is too small. The build runs the PGO training, so it takes a while; run it in the background.

When the `flathub` remote is in the user installation (`flatpak remotes`), use Flathub's own wrapper:

```bash
flatpak run --command=flathub-build org.flatpak.Builder <flathub-checkout>/net.andyofniall.missingno.json < /dev/null > build.log 2>&1
```

The wrapper hard-codes `--user --install-deps-from=flathub`. When the remote is system-wide, first install the Sdk and rust-stable extension for the target runtime with `flatpak install --system`, then pass the wrapper's flags directly, leaving those two out:

```bash
flatpak run org.flatpak.Builder --force-clean --disable-cache --sandbox --disable-rofiles-fuse --state-dir=state \
  --override-source-date-epoch 1321009871 --mirror-screenshots-url=https://dl.flathub.org/media \
  --compose-url-policy=full --repo=repo \
  builddir <flathub-checkout>/net.andyofniall.missingno.json < /dev/null > build.log 2>&1
```

Then lint the built repo the way Flathub's CI does:

```bash
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
```

The build passes when all of these hold:

- The build exits 0.
- The log shows `PGO build complete`.
- `builddir/files/share/metainfo/` contains the new release entry.
- The repo lint exits 0. Without `--mirror-screenshots-url` and `--compose-url-policy=full`, it fails on screenshot and icon mirroring.

## Step 6 — Hand over

Report:

- The release commit and tag, and that both are pushed.
- The Flathub branch and its uncommitted diff.
- The test-build result.

The developer commits on the Flathub branch, pushes it, and opens the PR into `master`.

Opening the next dev cycle (version bump, dependency update) is separate work. Offer it; don't start it.
