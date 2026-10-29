# R15 plan: command policy and transport details out of the adapters

Ticket Cherry-Blossom-Valley/drumspike#43, step R15. This is the plan only; no code until the
coordinator answers. Line references are against `origin/main` (`023966f`).

Gate (from the ticket): `git grep -n 'crate::bridge::' crates/app/src/call.rs` finds nothing,
`AppError` has typed variants instead of substring checks (`git grep -n 'contains("' crates/app/src/error*`
empty), and cross-surface contract tests (Tauri command and bridge) prove equal requests get
equal defaults, validation and mutation policy (counted test lines).

## today's state

Three adapters serve the same commands: the Tauri wrappers
(`apps/desktop/src-tauri/src/lib.rs`, 32 `#[tauri::command]`), the test bridge
(`crates/app/src/bridge.rs`, `invoke` 417 + `invoke_transport` 437-532) and the agent surface
(`crates/app/src/call.rs`: `call` 322 checks the registry, the arguments, the agent defaults
and the read-only guard, then `run` 42 dispatches). The command-line tool and the MCP server
call `crate::call` too (`cli.rs:28`, `mcp.rs:25` import only `Reply`).

1. **The dispatcher depends on the bridge.** `call.rs:16` imports `crate::bridge::{Channels,
   Reply}`; the same import lives in `copyjob.rs:40`, `lifecycle.rs:39`, `export.rs:28`,
   `relink.rs:28`, `tracks.rs:35`, `cli.rs:28`, `mcp.rs:25`. `Channels` (bridge.rs:229-278)
   and `Reply` (bridge.rs:190-193) are transport plumbing, not bridge concerns: every adapter
   points at the HTTP test bridge for them.

2. **Error classification is substring matching.** `error.rs:69-75` maps
   `ProjectError::Invalid(msg)` by `msg.contains("already exists") / "relink" / "cannot copy" /
   "no .drumspike document"`. The messages are produced in core
   (`project/save.rs:193` "project already exists", `save.rs:211` "relink it before Save As",
   `source.rs:342` "cannot copy source", and the missing-document check in the loader), so the
   policy lives in a string two crates away from the classifier: a wording change in core
   silently re-routes a 409 to 400. `ProjectError` (`core/src/project/mod.rs:65-94`) has only
   `Format`/`Invalid`/`Io`; the Python layer classifies by exception subclass, the Rust layer
   by text.

3. **Defaults, validation and the mutation policy are per-adapter.** Concretely:
   - `export_midi`: the Tauri wrapper hardcodes `compensate.unwrap_or(true)` (lib.rs:306);
     the bridge path parses via `ExportOpts::from_args` (call.rs:191, export.rs:390-402).
     Same value today, two owners of it.
   - `transport_play`: the Tauri wrapper builds `PlayWindow::from_args` itself (lib.rs:388-394)
     and so does `bridge.rs:461-467`; `transport_stop` defaults `close` in both
     (lib.rs:401, bridge.rs:478).
   - the agent defaults (`AGENT_DEFAULTS`, call.rs:236-240) and the argument check
     (`check_args`, call.rs:271) and the read-only guard (`changes_project` + registry check,
     call.rs:311-351) run only on the agent surface; the Tauri and bridge paths of the same
     commands skip all three (the Tauri wrapper of `analyze_start` defaults
     `keep_edited` itself, lib.rs:273). Nothing fails today — the page is trusted — but the
     contract "equal requests get equal defaults/validation/policy" is only true by
     convention, and no test pins it.

Constraints: the page and the e2e tests call the bridge and the wrappers unchanged; the
registry rows in `api.rs` stay the single source of the argument shapes; no behaviour change
is intended anywhere (the gate is about structure and proof, like #40).

## target shape

- A new `crates/app/src/reply.rs` owns `Channels` and `Reply` (moved verbatim from
  `bridge.rs:190-193, 229-278`, plus `QUEUE_CAP`). The bridge re-exports nothing;
  `bridge.rs` keeps `EventLog`, `HttpReply`, the server. `call.rs:16` becomes
  `use crate::reply::{Channels, Reply};` and the other seven imports follow. The gate grep
  hits zero.
- Typed `ProjectError` variants in `core/src/project/mod.rs` for the cases the strings carry
  today: `Exists(String)` ("already exists"), `NeedsRelink { track, reason }` (the Save-As
  source refusal), `CopyFailed { reason, path }` (`source.rs:342`), and a
  `NoDocument(String)` for the missing-document text. Each keeps its exact `Display` text
  (the tests in `project/tests.rs:99`, `save/tests.rs:400`, `source/tests.rs:290` pin the
  wording, so the page sees the same messages). `Invalid` stays for the rest.
  `error.rs` maps by variant (`matches!`), no `contains(`; the
  `project_errors_map_like_server_project_error` test (error.rs:123) extends to cover each
  new variant and that the strings are unchanged.
- One policy layer, `crates/app/src/policy.rs`: `check_args`, `changes_project`,
  `AGENT_DEFAULTS` and the read-only guard move here (from call.rs 236-318, `pub(crate)`),
  plus the small argument helpers (`arg_str`/`arg_i64`/`opt_i64`, call.rs:19-39) and a
  `transport_args` module for the shared `opt_f64`/`opt_bool`/`PlayWindow::from_args`
  parsing that `bridge.rs:439-448` and `lib.rs:388-394` duplicate. The Tauri wrappers keep
  their typed signatures (they are the API), but every default value and every refusal is
  decided in `policy.rs`; a wrapper that needs a default calls it.

## migration in small PRs

Each PR keeps main green: `cargo test -p drumspike-app -p drumspike-core --no-fail-fast`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and the e2e folder
(`pytest -m e2e_app --no-cov`) once on the last PR. No behaviour change in any PR.

1. **reply.rs** — move `Channels`/`Reply`, switch the eight imports. Bridge behaviour
   byte-identical; `bridge/tests.rs` passes untouched.
2. **typed ProjectError** — add the variants, produce them in core, map by variant in
   `error.rs`. The wording tests above prove no message changes; the e2e tests that assert
   codes/messages (error paths in tests/e2e_app) run unchanged.
3. **policy.rs** — move the check/defaults/guard code, point `call.rs`, the bridge and the
   wrappers' default decisions at it. Same PR adds the contract tests.

## the tests that prove the gate

`crates/app/src/policy/tests.rs` — the counted cross-surface contract lines. One table of
requests (command, args, surface) run twice: once through `call::call`/`run` (agent+bridge)
and once through the same `Service` methods the Tauri wrapper calls, asserting equal
`AppError.code`/message and equal result JSON for:

- equal defaults: `export_midi` without `compensate` (policy default true, both paths),
  `transport_stop` without `close`, `transport_play` without `start/end/loop/tracks/shot`
  (`PlayWindow::from_args` default), `analyze_start` without `keep_edited` on the agent
  surface (true) vs the page surface (the wrapper's own default, pinned so the two stay
  deliberately different or are unified — the plan asks the coordinator to decide which);
- equal validation: a wrong-typed and an unknown argument on a shared command answers 400
  with the key named on every surface that takes JSON args (the bridge always; the Tauri
  path via `check_args` in `policy.rs` once PR 3 lands);
- equal mutation policy: a mutating command on a read-only project answers 409 through the
  agent surface, the bridge and the wrapper path alike (`changes_project` from
  `policy.rs`).

Counted-line proof: the grep of the gate (empty results, pasted) plus the `cargo test -p
drumspike-app policy` counted output.

## risks

- `Channels`/`Reply` move touches eight files but is mechanical; `pub use` from `bridge` for
  one release would blur the gate, so no re-export — all users switch in PR 1.
- The typed error variants change core's public API; the Python-parity layer
  (`server.project_error` mapping, docs in `error.rs:61`) must keep the same codes — covered
  by the extended `project_errors_map_like_server_project_error`.
- Unifying the `keep_edited` default across surfaces would change page behaviour
  (the page sends the flag itself since P4-9, call.rs:234-235). Decided with the
  coordinator (#43/c-6481538632): the two defaults stay different, deliberately. The agent
  surface keeps `true` (an agent does not know about hand edits, so the safe default keeps
  them); the page always sends the flag itself (backend.js analyze: `!!keepEdited`, the
  user's choice; repick: `true`), so the wrapper's `unwrap_or(false)` (lib.rs:273) is never
  reached. The contract test pins both sides: the agent default is `true`, and the page
  calls send `keep_edited` explicitly, so the wrapper default stays unreachable.
