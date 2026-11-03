# Changelog

All notable changes to openshell-annotate. `scripts/release.sh <version>` moves the notes under "Unreleased" into a new
version section and adds the squash-merged PR titles since the previous tag. Versions follow [SemVer](https://semver.org).

## [Unreleased]

## [0.1.0-rc.1] - 2026-11-03

Merged pull requests:

- test: keep the golden policy independent of the crate version (#35)
- chore(release): tag-driven release workflow, packaging and installer (#33)
- feat(generate): propose fs read for image directories outside the baseline (--image) (#32)
- feat(lint): stop reporting URL literals in docstrings, messages and help text (#31)
- feat(generate): scaffold provider profile skeletons for unmapped credentials (#30)
- fix(providers): relocatable profile paths in providers.sh; note prover limit for tls=skip (#29)
- feat(providers): [providers] expect for dynamically looked-up credentials (#28)
- fix(generate): wrap long reasons instead of truncating them (#27)
- fix(diff-logs): find the repo root from subdirectories and keep probe scripts out of placement (#26)
- fix(scan): exclude patterns work with ./ paths, dead and regex patterns are reported (#25)
- feat(diff-logs): report connections that are allowed and then fail as TLS trust problems (#24)
- fix(lint): suggest one egress directive per host; fix invalid skill example (#22)
- feat(scan): read @openshell comments in Ruby, Rust, Java and more; report unscanned languages (#21)
- chore: issue templates, PR template and labels script (#6)
- chore(ci): test against newest OpenShell prerelease (#5)
- chore(ci): weekly upstream-compat build against OpenShell latest release and main (#4)
- feat: readable output for scan, diff-logs and generate (#3)
- feat(lint): readable multi-line output with context and category icons (#2)
- docs: developer guide, contributing workflow, clippy 1.95 fixes (#1)
