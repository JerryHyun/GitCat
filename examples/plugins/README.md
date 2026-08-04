# GitCat example plugins

Reference plugins for GitCat's plugin system. Each is a small, declarative
`plugin.json` manifest (plus a per-plugin README). They are intentionally
minimal, valid against the real manifest schema, and safe to install and read as
a starting point for your own.

A GitCat plugin is a `plugin.json` describing:

- **`commands`** — user-invokable actions surfaced in the ⌘K command palette.
- **`hooks`** — external commands GitCat runs when a lifecycle event fires.
- **`tama`** — an optional alternate look (and voice) for Tama: pose art, a
  greeting line, and a `voicePitch` (see [`midori-skin`](./midori-skin/)).
- **`lua`** — an optional main Luau script; a command/hook can name a `handler`
  function in it instead of a shell `run` (see [`lua-hello`](./lua-hello/)).

GitCat itself contacts **no AI and no network**. A plugin's `run` string is a
**user-authored external command** run on your machine — the same trust boundary
as a difftool/mergetool command. Install only plugins you trust.

## The examples

| Plugin | What it demonstrates |
| ------ | -------------------- |
| [`hello-tama`](./hello-tama/) | The smallest command; the `::gitcat.tama` reaction protocol (make Tama react from stdout). |
| [`commit-subject-lint`](./commit-subject-lint/) | A `commit-created` **hook** that lints the new commit's subject and reacts via Tama; plus an on-demand lint command. |
| [`open-in-editor`](./open-in-editor/) | External-tool commands using the **placeholder grammar** (`{repo}`, `{sha}`) and the `--` flag-injection guard. |
| [`midori-skin`](./midori-skin/) | A full-character **Tama skin**: the `tama` manifest field (eight `poses` + a `voicePitch` + a `copy` greeting), contributing no commands or hooks. |
| [`lua-hello`](./lua-hello/) | A **Luau-scripted** command: the `lua` manifest field + a command with a `handler` (not a shell `run`), using the sandboxed host API (`ctx`, `git`, `tama.react`, `print`). |

## Installing

Settings → **Plugins** tab → **Install plugin…** → pick a plugin's `plugin.json`
(or its folder). Toggle a plugin enabled/disabled or **Remove** it there.
Installed commands appear live in ⌘K; hooks fire automatically. The registry is
persisted in `plugins.json` under the app config dir.

## Manifest at a glance

```jsonc
{
  "id": "my-plugin",            // ^[a-z0-9][a-z0-9-]*$, unique in the registry
  "name": "My Plugin",          // required, non-empty
  "version": "1.0.0",           // required, non-empty
  "description": "…",           // optional
  "enabled": true,              // optional, defaults to true
  "commands": [
    {
      "id": "do-thing",
      "label": "Do the thing",
      "run": "mytool --repo {repo}", // external command TEMPLATE (non-empty)
      "context": "repo",   // none | commit | file | repo   (default: none)
      "placement": "palette" // palette | menu | both        (default: palette)
    }
  ],
  "hooks": [
    { "event": "commit-created", "run": "…" } // event is kebab-case
  ]
}
```

Honest caveats worth knowing:

- **`placement: "menu"`/`"both"` is not wired yet** (native context menu is
  backlog). A command with either placement currently appears **only in ⌘K**,
  exactly like `"palette"`. The examples all use `"palette"`.
- **`post-mutation` hooks are not fired yet** — the event exists in the schema
  but GitCat has no post-mutation chokepoint. Available events that DO fire:
  `repo-opened`, `repo-switched`, `pre-mutation`, `commit-created`, `undo`.
- Hooks are **fire-and-forget observers** — they cannot veto or block a GitCat
  operation.
- A plugin/hook command that changes the repo should declare **`"mutates": true`**:
  GitCat then snapshots before running it, so the change is covered by global
  **Undo** (a mutating action that omits `mutates` runs outside Undo).
- **Windows**: `run` lines are POSIX-shell one-liners; the Windows executor uses
  `cmd.exe`, and a value containing a `cmd` metacharacter (`& | < > ^ % ! "`,
  CR/LF) is refused fail-closed (so `{diff}` is usually refused on Windows).
  Assume a POSIX shell (Git-Bash / WSL) on PATH.

## More

See the full plugin documentation at [`docs/plugins.md`](../../docs/plugins.md).
