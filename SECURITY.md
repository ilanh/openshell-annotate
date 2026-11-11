# Security policy

openshell-annotate is an independent community tool; it is not affiliated with NVIDIA. Vulnerabilities in
OpenShell itself should be reported to the OpenShell project (see its `SECURITY.md`), not here.

## What this tool does and does not do

- It reads source files and configuration under the directories you point it at, and writes only where you tell it
  to (`generate -o`, `--providers-out`, `diff-logs --write`, `docs --out`).
- It makes no network calls and never executes the code it scans.
- It does not read or emit secret values. `provider` directives name environment variables only.

## Supported versions

The latest [pre-release](https://github.com/ilanh/openshell-annotate/releases) tag (currently **v0.1.0-rc.3**)
and the latest commit on `main`. Older pre-releases and arbitrary commits are best-effort only; please
reproduce on a supported ref when you can.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub's private vulnerability reporting
(the repository's **Security** tab, then **Report a vulnerability**). Include the version or commit, what you did,
and what you expected. You can expect an acknowledgement within a few days. Reports about a generated
policy that is broader than the directives justify, or a way to make the tool write outside its output paths,
are in scope.
