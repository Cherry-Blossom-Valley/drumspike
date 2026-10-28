---
# ENFORCED: crewboard reads these keys and refuses what they forbid
initiative: drumspike-cbv
owner: wfrank2509
coordinator_may_create_tickets: true
coordinator_may_split_tickets: true
coordinator_may_merge: true         # owner ruling 2026-10-16, kept for this repo: the coordinator merges itself
dev_may_open_prs: true
dev_branch_prefix: crew/
protected: [main]
ci_mode: local                      # crew/** pushes start no CI (#475): agents deliver with --local-run
# ADVICE: crewboard does not read these keys; the models read them as the owner's rules
needs_owner: [release, ci-config, credentials, delete, new-scope, new-repo]
---

# Authority of initiative drumspike-cbv

This file tells the agents what they may do. The owner (wfrank2509) edits it. Agents read it at
each `crewboard start` and obey the front matter.

## Scope

The initiative builds Drum Spike, the macOS app in this repository
(Cherry-Blossom-Valley/drumspike): detection, placement, editing, export, the guide and the
release tooling. The repository is public. Work arrives as tickets on the board "Drum Spike".

Not in scope: other repositories, the Cherry Blossom Valley website (cbv-deploy, Cbvsite), the
hosts and their runners, and anything on the `needs_owner` list.

## Rules for this repository

- One ticket, one branch `crew/<agent>/<issue>-<slug>`, one pull request with `Closes #<n>`.
- Agents gate locally (the ticket's Gate) and paste the counted result; CI on `main` runs after
  the merge. Heavy jobs run under the host lock (`crewboard lock run`).
- Never put a private name, host path, token or session file into the repository: it is public.
- A release is a tag the coordinator pushes after CI is green on `main`; the owner decides on
  publishing.

A PR that changes this file, `.crewboard.toml`, `.github/workflows/**`, `.github/actions/**` or
`.github/CODEOWNERS` is merged by the owner only, whatever the keys say.
