---
# ENFORCED: crewboard reads these keys and refuses what they forbid
initiative: drumspike-cbv
owner: wfrank2509
coordinator_may_create_tickets: true
coordinator_may_split_tickets: true
coordinator_may_merge: false        # true only if the owner allows it
dev_may_open_prs: true
dev_branch_prefix: crew/
protected: [main]
# protected_paths: [tests/**, scripts/ci-*.sh]  # more paths that only the owner merges; adds to the built-in five
# ci_ignore: [deploy-preview]       # CI jobs that are red for reasons outside a ticket; not counted
# ci_mode: local                    # the owner switches CI off: agents deliver with --local-run
# former_repos:                     # after a repo transfer or rename: the old name and an exclusive cutoff
#   - { name: old-owner/old-repo, until: 2026-11-01T12:00:00Z }
# ADVICE: crewboard does not read these keys; the models read them as the owner's rules
needs_owner: [release, credentials, delete, new-scope, new-repo]
---

# Authority of initiative drumspike-cbv

This file tells the agents what they may do. The owner (wfrank2509) edits it. Agents read it at
each `crewboard start` and obey the front matter. Describe the scope of the initiative here in
plain words: what the goal is, what is not in scope, which repos belong to it.

A PR that changes this file, `.crewboard.toml`, `.github/workflows/**`, `.github/actions/**` or
`.github/CODEOWNERS` is merged by the owner only, whatever the keys say.
