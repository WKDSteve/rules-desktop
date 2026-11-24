# Security of the local server

`rules-hub` is a web server on your own machine. Anything on the machine, and any web page you open in a browser, can try
to talk to it. The guard, the pairing, the sessions, the limits and the engine seam are those of
[appkit](https://github.com/WKDSteve/appkit): **its `SECURITY.md` is the description of what holds for every app, and
its security matrix (`appkit-testkit`, `security_matrix!`) is the shared test of it.** This hub runs that matrix
(`crates/rules-hub/tests/security.rs`): every route of `ROUTES` is a data route, and the matrix tries each one without a
session. This page says what is the rules hub's own.

## What is the rules hub's own

| Thing | What holds | Where it is tested |
| --- | --- | --- |
| Names | App, program, variables, port, routes and login entry are the hub's own and share none with `tasks-hub` or the demo app. | `tests/security.rs` (`no_name_of_the_rules_hub_is_a_name_of_another_app`) |
| Data routes | `snapshot`, `rule`, `sources`, `about`, `events`, `search`, `rescan`, `open`, `next_id`, `ops` all need a session; a route added to `ROUTES` is a data route by construction. | matrix; `tests/routes.rs` |
| The engine only reads | The table of calls has no write call (`ops` is not in it); the page cannot name a call: `bodies` (the search's own question) is not a route. | `src/engine.rs` tests; `tests/routes.rs` |
| No string from outside reaches the engine | `rule` and `open` take an id; an id that is not in the held snapshot is a 404 without a question to the engine. | `tests/routes.rs` |
| A rule set is code | The hub never runs a rule's `check`; the page shows it as JSON in a `<pre>`. | `e2e/app.spec.ts` |
| Rule text is text | Titles, texts, checks and sources are shown as text; hidden and bidi characters are made visible (`⟦U+202E⟧`). | `e2e/app.spec.ts` (a corpus of hostile rules: markup, script, `javascript:` links, event handlers, bidi) |
| Data classes fail closed | An unclassified rule set is marked, never released; an unreadable `sources.json` stops the start. | `src/sources.rs`; `tests/cli.rs` |
| Rule sets move only as the vault allows | `create_ruleset`, `archive_ruleset` and `restore_ruleset` take a folder name and no path; the engine builds the places inside the vault, refuses names that are not plain folder names, and never deletes (archive is a move to `archive/rulesets/`). The hub lets them through alone in a request, refuses to archive a rule set that has no class on this device (and one that the snapshot names differently from the folder), and reads the snapshot again after a change. | `tests/routes.rs`; `tests/cli.rs` (real engine: the folder moves, comes back byte for byte, an empty one is listed); `e2e/app.spec.ts` |
| Loosening a class is asked | `POST /api/rules/sources/class` writes `sources.json` itself (a file of the config folder, not a rule set): it checks the text with the rules of the file before it replaces the whole file, takes the new policy at once, and answers 409 `confirm_needed` for a change that releases more until the request carries `confirm_loosen`; the page asks the person and only then sends it again. A field it does not know, a bad key or class is a 400 and writes nothing. | `tests/routes.rs`; `e2e/app.spec.ts` (the question, "No" writes nothing) |
| Opening a file | `open` takes only an id. The place is the engine's to say (`rules.lua locate`: the file and the rule set that holds it; the name in `source` is not turned back into a place by the hub), and the file must be a plain file inside that rule set after links are resolved; the editor starts with a program and arguments, no shell. | `tests/routes.rs`; `src/editor.rs` |
| Writing rules | `ops` passes a request of `rules-ops/1` to the engine, the only writer. The hub checks its form and size first (known fields, 1 to 100 operations, `client_op_id`, `dry_run`), serves only `new`, `set`, `retire`, `reinstate` and `move` (the waiver operations write other repositories and are refused), counts a write against the limit of writes (a dry run costs nothing), and reads the snapshot again before it answers when something changed. A rule set with no class on this device (`sources.json`) is shown and never written: the hub works the class out from the `source` the held snapshot gives the rule (for a new rule, from the rule set it is made in), refuses the whole request with `write_refused` (403) if one operation is not allowed, answers 404 for an id the snapshot does not hold, and does not ask the engine in either case. The page never writes over a version it did not see: every change goes with the tag it saw, a clash opens the conflict center and nothing is written until the person chooses, and only the marked rules the page may write are sent in a bulk request. The waivers are written into the one project the person named in `config.json`: the hub puts that folder into each waiver operation itself and refuses a `project` that comes from the page, in any operation, before it judges anything else (a page with a session could otherwise make the engine write `.rules-waivers.json` into any folder of the machine). It formats and parses no rule; a rule set is named, never a path, so the page cannot point a write elsewhere; `if_match` and the engine's lock are the engine's. | `tests/routes.rs`, `tests/cli.rs` (real engine: only the line of the rule changes, the other file does not, the inverse restores it byte for byte) |
| Sessions end | Every view has a **Sign out** button: it posts to `/api/session/logout`, after which the token is dead and every open event stream of the session ends at once (not at the next heartbeat); the page then shows the pairing state, also when the hub does not answer. | `tests/routes.rs` (`an_open_stream_ends_when_its_session_signs_out`); `src/lib/app.test.ts`; `e2e/app.spec.ts` |
| The page's policy | `default-src 'none'`; scripts and styles from `self`; no inline script, no `unsafe-*`, Trusted Types. | `tests/routes.rs`; `e2e/app.spec.ts` |
| Nothing of a rule set in the browser | The session is in `sessionStorage` of the tab; `localStorage` holds language, theme and the single-key switch. No cookie is ever set. | `e2e/app.spec.ts` |
| Accessibility | axe reports nothing on every view, the detail and the palette, in the light and the dark scheme. | `e2e/app.spec.ts` |

## What it does not do

- It is not a defence against someone who already runs code as you: they can read `control.secret` and `sources.json`.
- The search index holds the rule texts in the memory of the hub; they are not written anywhere.
- It does not run the checks of rules and does not write rules.

## Reporting

A hole in the guard belongs to appkit (see its `SECURITY.md`); one in what this page lists belongs here. Open an issue
without the details, or write to the maintainer.
