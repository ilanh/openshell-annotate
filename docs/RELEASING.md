# Releasing and verifying

A release is created by pushing a version tag. `.github/workflows/release.yml` then builds, packages, attests and
publishes everything. Nothing is published from a branch or a pull request, and the workflow needs no secrets: it uses
the job's short-lived `GITHUB_TOKEN` (permissions are declared per job, `contents: read` by default).

## What a release contains

| Asset | Notes |
|---|---|
| `openshell-annotate-<version>-x86_64-unknown-linux-musl.tar.gz`, `...-aarch64-unknown-linux-musl.tar.gz` | static binaries, run on any Linux (glibc or musl) |
| `openshell-annotate-<version>-x86_64-apple-darwin.tar.gz`, `...-aarch64-apple-darwin.tar.gz` | macOS Intel and Apple silicon (the Intel one is cross-compiled on an arm64 runner) |
| `openshell-annotate-<version>.spdx.json` | SBOM (SPDX) generated from `Cargo.lock` with syft |
| `SHA256SUMS` | SHA-256 of every file above |
| `ghcr.io/ilanh/openshell-annotate:<version>` | multi-arch (amd64, arm64) image on distroless/static that contains the same two Linux binaries; `:latest` moves only for stable releases |

Each tarball holds `openshell-annotate-<version>-<target>/{openshell-annotate,LICENSE,README.md}` and is built
deterministically (sorted entries, fixed mtime, `gzip -n`), so packaging the same binary twice gives the same bytes.
The tarballs, SBOM, `SHA256SUMS` and the image carry a GitHub build provenance attestation
(`actions/attest-build-provenance`): a signed statement, recorded in GitHub's attestation store and the public Sigstore
log, that this repository's `release.yml` produced exactly those bytes from the tagged commit.

## Cutting a release

1. Start from an up-to-date `main`. Make sure `CHANGELOG.md` has any hand-written notes under `## [Unreleased]`.
2. `scripts/release.sh 0.2.0` (or `0.2.0-rc.1`). It bumps `Cargo.toml` and `Cargo.lock`, moves the unreleased notes
   into a new `## [0.2.0]` section together with the squash-merged PR titles since the previous tag, and prints the next
   commands. It edits files only: it never commits, tags or pushes.
3. Review `git diff`, then open a PR from a `chore/release-v0.2.0` branch (`main` is protected) and merge it.
4. Tag the merged commit on `main` and push the tag. This is the step that publishes:
   ```
   git checkout main && git pull --ff-only
   git tag -s v0.2.0 -m "v0.2.0"        # or `git tag -a` if you do not sign tags
   git push origin v0.2.0
   ```
5. Watch the run under Actions. When it is green the release, assets and image exist.

Tag rules, enforced by the `plan` job (it fails before anything is built or published):

- the tag must look like `vMAJOR.MINOR.PATCH` or `vMAJOR.MINOR.PATCH-<prerelease>` (`v0.2.0-rc.1`, `v0.2.0-pre.3`);
- the tag version must equal `version` in `Cargo.toml`;
- a tag with a `-` suffix is published as a **prerelease** (not "latest", and the image does not get `:latest`);
- a stable tag must point at a commit that is on `main`. Prerelease tags may be cut from a branch, which is how you test
  the pipeline with a real artifact without touching `latest`.

If a run fails after the image was pushed, re-run the failed jobs: pushing the same version tag again to the image is
idempotent, and `gh release create` is the last step. To abandon a bad release, delete the GitHub release and the tag,
fix forward and publish a new version; do not reuse a version number that people may have downloaded.

Pull requests that touch the release workflow, packaging scripts, `Dockerfile.release`, `install.sh` or `Cargo.*` run
the same builds as a dry run (no login, no push, no attestation, no release), so changes to the pipeline are tested
before a tag exists.

### One-time setup

- The first image push creates the `ghcr.io/ilanh/openshell-annotate` package as **private**. In the repository's
  Packages settings, link it to the repository (the `org.opencontainers.image.source` label does this automatically) and
  change its visibility to public so that anyone can `docker pull` it.
- Actions must be allowed to create attestations (the default for public repositories). If you restrict allowed actions,
  permit `actions/*`, `docker/*` and `anchore/sbom-action`. All actions are pinned to a commit SHA.

## Promoting `openshell-next` when OpenShell ships stable

`openshell-next` (see `docs/DEVELOPING.md`, "Tracking the next OpenShell release") builds against OpenShell
prereleases. When upstream tags the stable `vX.Y.Z`, the daily workflow moves the branch to it and adds a "stable
release" note to the tracking issue. To bring it to `main`:

1. Read the upstream release notes and the compare link in that comment; tick the tracking PR's checklist.
2. On `openshell-next` (via a `chore/openshell-vX.Y.Z` PR into it, or directly if you maintain the branch), update
   everything that names the old version: `git grep -n 'v\?0\.1\.[0-9]'` (README, AGENTS.md, docs, `Dockerfile`, the strings
   in `src/`), regenerate the goldens (`UPDATE_GOLDEN=1 cargo test`, then review), and add a CHANGELOG line under
   `## [Unreleased]`. The workflow only moves the pin, so these strings still name the old version until then.
3. Retitle the tracking PR to `chore(deps): bump OpenShell crates to vX.Y.Z`, put `Closes #<tracking issue>` in it,
   mark it ready for review, and squash-merge it once `check` and `audit` pass. `openshell-next` itself survives the
   merge (the `protect-openshell-next` ruleset blocks the automatic branch deletion) and keeps tracking from `main`.
4. Open a new tracking issue (label `openshell-next`) and a new draft PR for the following release, or let the
   workflow create the issue at the next upstream tag; it links a compare view for opening the PR.
5. Cut a release of openshell-annotate as usual if users should get the new pin.

Never promote a prerelease pin to `main`: stable openshell-annotate releases build against stable OpenShell only.

## Installing

```
curl -fsSL https://raw.githubusercontent.com/ilanh/openshell-annotate/main/install.sh | sh
curl -fsSL https://raw.githubusercontent.com/ilanh/openshell-annotate/main/install.sh | sh -s -- --version v0.2.0-rc.1 --dir /usr/local/bin
```

`install.sh` picks the right archive for the OS and CPU, downloads `SHA256SUMS` next to it and refuses to install on a
mismatch. A checksum fetched from the same place as the archive only detects a corrupted download; for trust in where
the binary came from use `--verify` (needs the GitHub CLI) or verify by hand as below. Default install directory is
`~/.local/bin`; "latest" means the newest stable release (prereleases need `--version`).

## Verifying a download

Everything below needs only the file and the [GitHub CLI](https://cli.github.com) (`gh` 2.49 or newer).

```
# the archive was built by this repository's release workflow, from the tagged commit
gh attestation verify openshell-annotate-0.2.0-x86_64-unknown-linux-musl.tar.gz --repo ilanh/openshell-annotate

# stricter: pin the signing workflow too
gh attestation verify openshell-annotate-0.2.0-x86_64-unknown-linux-musl.tar.gz \
  --repo ilanh/openshell-annotate \
  --signer-workflow ilanh/openshell-annotate/.github/workflows/release.yml

# checksums of everything you downloaded (SHA256SUMS is itself attested)
gh attestation verify SHA256SUMS --repo ilanh/openshell-annotate
sha256sum --check --ignore-missing SHA256SUMS        # macOS: shasum -a 256 --check --ignore-missing SHA256SUMS

# the container image
gh attestation verify oci://ghcr.io/ilanh/openshell-annotate:0.2.0 --repo ilanh/openshell-annotate
```

`gh attestation verify --format json` prints the signed statement: the source repository, the commit SHA, the tag
(`refs/tags/v0.2.0`) and the workflow run that built the file.

## Building the same thing locally

```
cargo build --release --locked                 # whatever your host is
scripts/smoke.sh target/release/openshell-annotate
scripts/package.sh v0.2.0 x86_64-unknown-linux-gnu target/release/openshell-annotate dist
```

The release targets are built on GitHub's runners with Rust 1.94: on Ubuntu with `musl-tools` and
`CC_<target>=musl-gcc` for the static Linux binaries (the tree-sitter grammars are C), and with the stock toolchain on macOS.
The workflow passes `--remap-path-prefix` so no build paths end up in the binary.
