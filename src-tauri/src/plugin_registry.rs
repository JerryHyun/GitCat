//! Plugin system — backend foundation (PER-39). GitCat plugins are small,
//! declarative manifests: a `plugin.json` describing external-command
//! `commands` (surfaced in the ⌘K palette and/or context menus) and `hooks`
//! (external commands GitCat runs on lifecycle events). This module owns ONLY
//! the on-disk registry + install/enable/remove CRUD; a later executor module
//! consumes [`load_plugins`]/[`find_command`] to actually run a command's
//! external process.
//!
//! ## AI-agnostic / trust boundary
//!
//! GitCat itself never runs a plugin's `run` string here — this module only
//! parses, validates, and persists manifests. Exactly like
//! `tool_settings.rs`'s diff/merge `cmd` and `commit_msg_command`, a plugin's
//! `run` is user-authored shell text with the SAME trust boundary (a command
//! the user installed for their OWN machine). GitCat connects to no AI and
//! invents no commands; whatever a plugin does lives entirely inside its own
//! `run` strings. Validation here is about manifest well-formedness (a stable
//! `id`, present name/version, non-empty run), NOT sandboxing.
//!
//! ## Persistence
//!
//! `plugins.json` under Tauri's `app_config_dir()` — a single small versioned
//! JSON file, EXACT mirror of `repo_registry.rs`/`tool_settings.rs`: same
//! `SCHEMA_VERSION`, same corrupt-file rename-aside recovery
//! (`plugins.json.corrupt-<unix>`), same same-directory temp+rename atomic
//! write, same process-wide poison-recovered `Mutex` serializing every
//! read-modify-write. The pure persistence layer ([`load_from`]/[`save_to`])
//! and the pure validation/mutation helpers ([`read_and_validate_manifest`],
//! [`install_from`], [`set_enabled_in`], [`remove_from`]) take a plain `&Path`
//! (never an `AppHandle`) so the unit tests below drive the real logic against
//! a throwaway temp file, exactly like `repo_registry.rs`/`tool_settings.rs`
//! do. The `#[tauri::command]`s are thin `AppHandle`-taking wrappers.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Wry};

const FILE_NAME: &str = "plugins.json";
const SCHEMA_VERSION: u32 = 1;

/// Hard cap on a manifest file's size. A `plugin.json` is a tiny declarative
/// document (an id, name, version, and a handful of command/hook strings) —
/// anything past a quarter-megabyte is not a legitimate manifest, so reject it
/// up front rather than reading arbitrarily large bytes into memory and
/// handing them to the JSON parser.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// The eight built-in Tama pose keys — an EXACT mirror of the frontend's
/// `TAMA_IMG` map (`src/legacy/main.ts`). A plugin skin may only OVERRIDE one
/// of these poses; any other key is rejected at [`validate_manifest`] time
/// (see [`PluginTama`]). Kept in sync by hand with the frontend map.
const VALID_TAMA_POSE_KEYS: [&str; 8] =
    ["hero", "curious", "confident", "thinking", "happy", "alarm", "shocked", "sleep"];

/// Hard cap on a single Tama pose asset's size, enforced by [`load_plugin_skin`]
/// before any bytes are read. A pose sprite is a small optimized WebP/PNG;
/// anything past half a megabyte is not a legitimate pose image, so skip it
/// rather than base64-inline a huge blob into a `data:` URI.
const MAX_TAMA_ASSET_BYTES: u64 = 512 * 1024;

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

/// What a command needs from the current UI selection to be applicable — the
/// executor uses this to decide where a command is offered (e.g. a `commit`
/// command only in a commit's context menu). Serialized lowercase so a
/// manifest author writes `"context": "commit"`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum PluginContext {
    /// No selection needed — always applicable (the default when omitted).
    #[default]
    None,
    /// Needs a selected commit.
    Commit,
    /// Needs a selected file.
    File,
    /// Repo-scoped (needs only the open repository).
    Repo,
}

/// Where a command surfaces. Serialized lowercase (`"palette"`/`"menu"`/
/// `"both"`); defaults to `Palette` when a manifest omits it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum PluginPlacement {
    /// Only in the ⌘K command palette (the default).
    #[default]
    Palette,
    /// Only in the relevant context menu.
    Menu,
    /// Both the palette and the context menu.
    Both,
}

/// A lifecycle event a hook can fire on. Serialized kebab-case
/// (`"repo-opened"`, `"pre-mutation"`, …).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum PluginEvent {
    RepoOpened,
    RepoSwitched,
    PreMutation,
    PostMutation,
    CommitCreated,
    Undo,
}

/// One user-invokable command a plugin contributes. `run` is an external
/// command TEMPLATE (user-authored shell text — see the module doc's trust
/// note); the executor expands its placeholders and runs it. `context`/
/// `placement` both `#[serde(default)]` so a minimal manifest command needs
/// only `id`/`label`/`run`.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginCommand {
    pub id: String,
    pub label: String,
    /// External command TEMPLATE. Validated non-empty at install time (see
    /// [`validate_manifest`]); NOT otherwise sanitized — same trust boundary
    /// as `tool_settings.rs`'s diff/merge `cmd`.
    pub run: String,
    #[serde(default)]
    pub context: PluginContext,
    #[serde(default)]
    pub placement: PluginPlacement,
    /// Does this command CHANGE the repository? A plugin DECLARES `true` for a
    /// command that mutates (checks out, resets, commits, deletes files, …).
    /// `#[serde(default)]` => `false` when omitted, so a v1 manifest with no
    /// `mutates` still loads and is treated as read-only.
    ///
    /// The executor honors it: for a `mutates: true` invocation it takes a
    /// `crate::safety::snapshot` BEFORE running `run`, so the change enters
    /// global Undo; a `mutates: false` invocation takes NO snapshot (no Undo
    /// clutter for read-only tools). A mutating command that does NOT set this
    /// runs OUTSIDE Undo — authors must set it (see SECURITY.md and
    /// `plugin_exec::run_plugin_command`).
    #[serde(default)]
    pub mutates: bool,
}

/// One lifecycle hook: run `run` (an external command TEMPLATE, same trust
/// boundary as [`PluginCommand::run`]) when `event` fires.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginHook {
    pub event: PluginEvent,
    pub run: String,
    /// Does this hook CHANGE the repository? Same semantics as
    /// [`PluginCommand::mutates`] (default `false` => pure observer, no
    /// snapshot). A `mutates: true` hook is snapshotted before it runs so its
    /// change is covered by global Undo; see `plugin_exec::run_hooks`.
    #[serde(default)]
    pub mutates: bool,
}

/// A plugin-contributed Tama SKIN (PER-47): pose sprites that override the
/// built-in cat art, plus optional copy/voice lines. `poses` maps a built-in
/// pose KEY (one of [`VALID_TAMA_POSE_KEYS`]) to a RELATIVE asset path within
/// the plugin's own directory (e.g. `"curious" -> "assets/curious.webp"`).
/// Both the key set and the relative-path safety are enforced up front in
/// [`validate_manifest`]; the actual bytes are loaded — with a second,
/// canonicalizing containment guard — by [`load_plugin_skin`].
#[derive(Serialize, Deserialize, Clone, Debug, Default, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginTama {
    /// Pose key -> relative asset path within the plugin dir. Every key must be
    /// one of [`VALID_TAMA_POSE_KEYS`]; every value must be a relative path with
    /// no `..` and not absolute (validated at install time).
    pub poses: HashMap<String, String>,
    /// Optional voice/copy lines the skin contributes, passed through verbatim.
    #[serde(default)]
    pub copy: HashMap<String, String>,
}

/// A plugin manifest (`plugin.json`).
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// Stable identifier, `^[a-z0-9][a-z0-9-]*$` (enforced at install — see
    /// [`is_valid_id`]). Unique within the registry; used as the remove/enable
    /// key and half of a command's `(pluginId, commandId)` address.
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    /// Defaults to `true` when a manifest omits it — a freshly installed
    /// plugin is active unless the user disables it.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
    #[serde(default)]
    pub hooks: Vec<PluginHook>,
    /// Optional Tama SKIN (PER-47) — pose sprites + copy this plugin
    /// contributes. `#[serde(default)]` + `Option` so every pre-skin manifest
    /// still loads (absent => no skin). See [`PluginTama`].
    #[serde(default)]
    pub tama: Option<PluginTama>,
    /// The plugin's on-disk SOURCE directory (the canonicalized PARENT of its
    /// `plugin.json`), captured at install time so a later skin load knows
    /// where to resolve relative asset paths. `#[serde(default)]` + `Option`:
    /// pre-PER-47 registries have none, and a plugin whose source dir could not
    /// be resolved at install (e.g. a non-canonicalizable path) simply has its
    /// Tama assets UNAVAILABLE — the skin loads with an empty `poses` rather
    /// than erroring. Persisted verbatim in `plugins.json`.
    #[serde(default)]
    pub dir: Option<String>,
}

fn default_true() -> bool {
    true
}

/// On-disk shape. `plugins` is `#[serde(default)]` so a `{"version":1}`-only
/// file (or a future field-only bump) still deserializes to an empty list
/// rather than tripping corrupt-file recovery.
#[derive(Serialize, Deserialize)]
struct PluginsFile {
    version: u32,
    #[serde(default)]
    plugins: Vec<Plugin>,
}

// ---------------------------------------------------------------------------
// Persistence (mirrors repo_registry.rs / tool_settings.rs line-for-line)
// ---------------------------------------------------------------------------

fn plugins_path(app: &AppHandle<Wry>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Could not resolve app config dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create app config dir: {e}"))?;
    Ok(dir.join(FILE_NAME))
}

/// Read the plugins file at `path`. Missing file => empty list (first run),
/// never an error. A malformed/corrupt file is renamed aside to
/// `plugins.json.corrupt-<unix-seconds>` (best-effort — if even that fails,
/// e.g. a read-only directory, we proceed rather than compounding one failure
/// into a second) and an empty list is returned, exactly like a first run:
/// the corrupt bytes survive on disk for forensics but never permanently lock
/// the user out. Identical recovery story to `repo_registry::load_from`.
///
/// `pub` for the same integration-testability reason as
/// `repo_registry::load_from`: the tests drive the real persistence logic
/// against a throwaway temp file without a real `AppHandle`.
pub fn load_from(path: &Path) -> Result<Vec<Plugin>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Could not read {}: {e}", path.display())),
    };
    match serde_json::from_str::<PluginsFile>(&text) {
        Ok(file) => Ok(file.plugins),
        Err(_) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup = path.with_file_name(format!(
                "{}.corrupt-{now}",
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| FILE_NAME.to_string())
            ));
            let _ = std::fs::rename(path, &backup); // best-effort; proceed regardless
            Ok(Vec::new())
        }
    }
}

/// Process-wide lock serializing every registry read-modify-write sequence —
/// identical rationale/shape to `repo_registry::registry_lock`: without it two
/// concurrent writers each doing an unlocked load -> mutate -> save could let
/// "last write wins" silently drop the loser's change. A poisoned lock (a
/// prior panic mid-critical-section) is recovered from rather than propagated.
fn plugins_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// `pub` for the same integration-testability reason as [`load_from`]. Writes
/// via a same-directory temp file + atomic rename (never a direct in-place
/// `fs::write`) so a crash/power-loss mid-write can never leave a half-written,
/// corrupt `plugins.json` behind.
pub fn save_to(path: &Path, plugins: &[Plugin]) -> Result<(), String> {
    let file = PluginsFile { version: SCHEMA_VERSION, plugins: plugins.to_vec() };
    let json = serde_json::to_string_pretty(&file).map_err(|e| format!("Could not serialize: {e}"))?;
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = PathBuf::from(tmp_name);
    std::fs::write(&tmp_path, &json).map_err(|e| format!("Could not write {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path).map_err(|e| format!("Could not finalize {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Validation + pure mutations (AppHandle-free, directly unit-tested)
// ---------------------------------------------------------------------------

/// `^[a-z0-9][a-z0-9-]*$` — first char a lowercase ASCII letter or digit, the
/// rest lowercase letters, digits, or `-`. Enforced once at install time so
/// an `id` is always a safe, stable key (used verbatim as the remove/enable
/// key and as half of a `(pluginId, commandId)` command address).
fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Validate a parsed manifest's well-formedness: valid `id` charset, non-empty
/// name/version, and every command's `run` non-empty. Does NOT check id
/// uniqueness (that needs the registry — see [`install_from`]). `pub` so the
/// tests exercise it directly.
pub fn validate_manifest(plugin: &Plugin) -> Result<(), String> {
    if !is_valid_id(&plugin.id) {
        return Err(format!(
            "Plugin id {:?} is invalid — it must start with a lowercase letter or digit and then contain only lowercase letters, digits, and '-'.",
            plugin.id
        ));
    }
    if plugin.name.trim().is_empty() {
        return Err("Plugin manifest is missing a non-empty name.".into());
    }
    if plugin.version.trim().is_empty() {
        return Err("Plugin manifest is missing a non-empty version.".into());
    }
    for cmd in &plugin.commands {
        if cmd.run.trim().is_empty() {
            return Err(format!("Plugin command {:?} has an empty run command.", cmd.id));
        }
    }
    // Tama skin (PER-47): every pose key must be a built-in pose, and every
    // asset path must be a SAFE relative path. This is the up-front, string-
    // level gate; [`load_plugin_skin`] additionally canonicalizes + containment-
    // checks each asset (catching symlink escape) before reading any bytes.
    if let Some(tama) = &plugin.tama {
        for (key, rel) in &tama.poses {
            if !VALID_TAMA_POSE_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "Plugin tama pose key {key:?} is not a built-in pose — it must be one of {VALID_TAMA_POSE_KEYS:?}.",
                ));
            }
            if Path::new(rel).is_absolute() || rel.contains("..") {
                return Err(format!(
                    "Plugin tama pose {key:?} has an unsafe asset path {rel:?} — it must be a relative path inside the plugin dir (no leading '/' and no '..').",
                ));
            }
        }
    }
    Ok(())
}

/// Resolve `source` (a `plugin.json` file OR a directory containing one) to the
/// manifest file, enforce the [`MAX_MANIFEST_BYTES`] size cap, then read,
/// parse, and [`validate_manifest`] it. Returns the parsed [`Plugin`] WITHOUT
/// touching the registry (no uniqueness check, no save) — [`install_from`]
/// layers those on top. `pub` for direct unit testing.
pub fn read_and_validate_manifest(source: &Path) -> Result<Plugin, String> {
    let manifest = if source.is_dir() { source.join("plugin.json") } else { source.to_path_buf() };
    let meta = std::fs::metadata(&manifest).map_err(|e| format!("Could not read {}: {e}", manifest.display()))?;
    // Must be a REGULAR file. A FIFO/device/symlink-to-device reports len()==0
    // (slipping past the size cap) and could then block indefinitely or stream
    // unbounded bytes into the parser.
    if !meta.is_file() {
        return Err(format!("Plugin manifest {} is not a regular file.", manifest.display()));
    }
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "Plugin manifest {} is too large ({} bytes; the limit is {MAX_MANIFEST_BYTES} bytes).",
            manifest.display(),
            meta.len()
        ));
    }
    // Read through a hard byte cap (belt-and-suspenders against a TOCTOU grow
    // between the metadata check and the read): never hand more than the cap to
    // the parser, even if the file changed size since the check above.
    use std::io::Read;
    let file = std::fs::File::open(&manifest).map_err(|e| format!("Could not read {}: {e}", manifest.display()))?;
    let mut text = String::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("Could not read {}: {e}", manifest.display()))?;
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(format!("Plugin manifest {} is too large (limit {MAX_MANIFEST_BYTES} bytes).", manifest.display()));
    }
    let mut plugin: Plugin =
        serde_json::from_str(&text).map_err(|e| format!("{} is not a valid plugin manifest: {e}", manifest.display()))?;
    validate_manifest(&plugin)?;
    // Capture the plugin's SOURCE directory (PER-47): the CANONICALIZED parent
    // of its manifest file, so a later skin load resolves relative asset paths
    // against a stable, symlink-free root. This ALWAYS overwrites any `dir` the
    // manifest JSON itself carried — `dir` is GitCat-authoritative and must
    // NEVER be attacker-controlled (a manifest declaring `"dir":"/etc"` must not
    // steer skin loads at some arbitrary directory). Best-effort: if the parent
    // can't be canonicalized `dir` becomes None (the manifest is otherwise valid
    // and installs fine; its Tama assets are simply unavailable).
    plugin.dir = std::fs::canonicalize(&manifest)
        .ok()
        .and_then(|canon| canon.parent().map(|d| d.to_path_buf()))
        .map(|d| d.to_string_lossy().into_owned());
    Ok(plugin)
}

/// Read+validate the manifest at `source`, reject a duplicate `id` against the
/// registry at `plugins_path`, then append + save. Returns the installed
/// plugin. `pub` + `&Path`-taking (no `AppHandle`) so the tests drive the full
/// install path directly; the command wraps it under [`plugins_lock`].
pub fn install_from(plugins_path: &Path, source: &Path) -> Result<Plugin, String> {
    let plugin = read_and_validate_manifest(source)?;
    let mut plugins = load_from(plugins_path)?;
    if plugins.iter().any(|p| p.id == plugin.id) {
        return Err(format!("A plugin with id {:?} is already installed.", plugin.id));
    }
    plugins.push(plugin.clone());
    save_to(plugins_path, &plugins)?;
    Ok(plugin)
}

/// Set `enabled` on the plugin with `id` in place. Errs (rather than silently
/// no-oping) when no such plugin exists, so a stale UI id surfaces clearly.
/// `pub` for direct unit testing.
pub fn set_enabled_in(plugins: &mut [Plugin], id: &str, enabled: bool) -> Result<(), String> {
    match plugins.iter_mut().find(|p| p.id == id) {
        Some(p) => {
            p.enabled = enabled;
            Ok(())
        }
        None => Err(format!("No plugin with id {id:?} is installed.")),
    }
}

/// Remove the plugin with `id`. Errs when no such plugin exists. `pub` for
/// direct unit testing.
pub fn remove_from(plugins: &mut Vec<Plugin>, id: &str) -> Result<(), String> {
    let before = plugins.len();
    plugins.retain(|p| p.id != id);
    if plugins.len() == before {
        return Err(format!("No plugin with id {id:?} is installed."));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public helpers for the executor module
// ---------------------------------------------------------------------------

/// Load the whole plugin registry. The executor module calls this to enumerate
/// contributed commands/hooks. Returns every plugin regardless of `enabled` —
/// the caller decides whether to honor a disabled one.
pub fn load_plugins(app: &AppHandle<Wry>) -> Result<Vec<Plugin>, String> {
    load_from(&plugins_path(app)?)
}

/// Look up a single command by its `(pluginId, commandId)` address, the way the
/// executor resolves a palette/menu invocation back to its `run` template.
/// `Ok(None)` when either the plugin or the command doesn't exist. Does NOT
/// filter on `Plugin::enabled` — the caller checks that if it cares.
pub fn find_command(app: &AppHandle<Wry>, plugin_id: &str, command_id: &str) -> Result<Option<PluginCommand>, String> {
    let plugins = load_plugins(app)?;
    Ok(plugins
        .into_iter()
        .find(|p| p.id == plugin_id)
        .and_then(|p| p.commands.into_iter().find(|c| c.id == command_id)))
}

// ---------------------------------------------------------------------------
// Tama skin loading (PER-47)
// ---------------------------------------------------------------------------

/// A loaded Tama SKIN, ready for the frontend: `poses` maps each built-in pose
/// key to a `data:` URI (`data:image/webp;base64,...` or `image/png`) usable
/// directly as an `<img>` src, and `copy` carries the skin's optional
/// voice/copy lines. Only poses whose asset passed every safety check appear —
/// a missing/oversize/wrong-extension/escaping asset is silently omitted, so a
/// partial skin still loads.
#[derive(Serialize, Deserialize, Clone, Debug, Default, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TamaSkin {
    pub poses: HashMap<String, String>,
    pub copy: HashMap<String, String>,
}

/// Map an allow-listed asset extension to its `data:` URI MIME type. Returns
/// `None` for anything but `.webp`/`.png` (case-insensitive), which is how the
/// loader ENFORCES the extension allow-list — an unlisted extension yields no
/// MIME and the asset is skipped.
fn tama_asset_mime(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("webp") => Some("image/webp"),
        Some("png") => Some("image/png"),
        _ => None,
    }
}

/// SECURITY-CRITICAL containment guard. Given a CANONICALIZED plugin `dir` and a
/// CANONICALIZED resolved `asset`, is the asset genuinely inside the dir? Both
/// paths MUST already be canonicalized by the caller — that is what makes this
/// single `starts_with` block BOTH `..` traversal AND symlink escape: a symlink
/// under the plugin dir that points at `/etc/passwd` canonicalizes to
/// `/etc/passwd`, which does not start with the canonical plugin dir, so it's
/// rejected. `Path::starts_with` is component-wise, so `/plugins/foo` is NOT a
/// prefix of a sibling `/plugins/foobar`. Pure + directly unit-tested.
fn asset_is_within_dir(canonical_dir: &Path, canonical_asset: &Path) -> bool {
    canonical_asset.starts_with(canonical_dir)
}

/// Build a [`TamaSkin`] from `plugin`'s declared Tama assets. PURE (no
/// `AppHandle`) so it's driven directly by the unit tests. Does file IO —
/// callers run it on the blocking pool (see [`load_plugin_skin`]).
///
/// `copy` passes straight through from the manifest. For each declared pose,
/// the asset is admitted ONLY if it clears every gate: an allow-listed
/// extension (`.webp`/`.png`); a relative, non-`..` declared path; its resolved
/// path canonicalizes and, per [`asset_is_within_dir`], stays inside the
/// canonical plugin dir (blocking traversal AND symlink escape); it is a
/// regular file within [`MAX_TAMA_ASSET_BYTES`]. Any asset failing a gate is
/// SKIPPED (logged), never fatal — nothing outside the plugin dir is ever read.
/// A plugin with no [`Plugin::tama`] or no resolvable [`Plugin::dir`] yields an
/// empty (or copy-only) skin.
pub fn build_skin(plugin: &Plugin) -> TamaSkin {
    let mut skin = TamaSkin::default();
    let Some(tama) = plugin.tama.as_ref() else {
        return skin; // no skin declared
    };
    skin.copy = tama.copy.clone();

    let Some(dir) = plugin.dir.as_deref() else {
        eprintln!("plugin {:?}: Tama assets unavailable — no resolvable source dir", plugin.id);
        return skin; // copy-only; assets unavailable (documented)
    };
    let canonical_dir = match std::fs::canonicalize(dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("plugin {:?}: cannot canonicalize source dir {dir:?}: {e}", plugin.id);
            return skin;
        }
    };

    for (key, rel) in &tama.poses {
        // Defense in depth: validate_manifest already enforced these, but the
        // loader NEVER trusts that — re-check the key and the string-level path
        // safety before touching the filesystem.
        if !VALID_TAMA_POSE_KEYS.contains(&key.as_str()) {
            continue;
        }
        let rel_path = Path::new(rel);
        if rel_path.is_absolute() || rel.contains("..") {
            continue;
        }
        let Some(mime) = tama_asset_mime(rel_path) else {
            continue; // extension not in the allow-list
        };

        let resolved = canonical_dir.join(rel_path);
        let canonical_asset = match std::fs::canonicalize(&resolved) {
            Ok(a) => a,
            Err(_) => continue, // missing / unreadable
        };
        // SECURITY-CRITICAL: reject anything that canonicalizes OUTSIDE the dir.
        if !asset_is_within_dir(&canonical_dir, &canonical_asset) {
            eprintln!("plugin {:?}: skipping pose {key:?} — asset escapes the plugin dir", plugin.id);
            continue;
        }

        let meta = match std::fs::metadata(&canonical_asset) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() || meta.len() > MAX_TAMA_ASSET_BYTES {
            continue; // not a regular file, or over the per-asset cap
        }
        let bytes = match std::fs::read(&canonical_asset) {
            Ok(b) => b,
            Err(_) => continue,
        };
        // Belt-and-suspenders against a TOCTOU grow between metadata and read.
        if bytes.len() as u64 > MAX_TAMA_ASSET_BYTES {
            continue;
        }

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        skin.poses.insert(key.clone(), format!("data:{mime};base64,{b64}"));
    }
    skin
}

// ---------------------------------------------------------------------------
// Tauri commands (sync, like repo_registry.rs's)
// ---------------------------------------------------------------------------

/// JS: `commands.listPlugins()`.
#[tauri::command]
#[specta::specta]
pub fn list_plugins(app: AppHandle<Wry>) -> Result<Vec<Plugin>, String> {
    load_plugins(&app)
}

/// Enable/disable an installed plugin. JS: `commands.setPluginEnabled(id, enabled)`.
#[tauri::command]
#[specta::specta]
pub fn set_plugin_enabled(app: AppHandle<Wry>, id: String, enabled: bool) -> Result<(), String> {
    let _guard = plugins_lock().lock().unwrap_or_else(|e| e.into_inner());
    let store = plugins_path(&app)?;
    let mut plugins = load_from(&store)?;
    set_enabled_in(&mut plugins, &id, enabled)?;
    save_to(&store, &plugins)
}

/// Install a plugin from a local `plugin.json` file OR a directory containing
/// one: read + parse + validate, reject a duplicate id, then append + save.
/// Returns the installed plugin. JS: `commands.installPluginFromPath(path)`.
#[tauri::command]
#[specta::specta]
pub fn install_plugin_from_path(app: AppHandle<Wry>, path: String) -> Result<Plugin, String> {
    let _guard = plugins_lock().lock().unwrap_or_else(|e| e.into_inner());
    let store = plugins_path(&app)?;
    install_from(&store, Path::new(&path))
}

/// Uninstall a plugin (removes it from the registry only; never touches the
/// original manifest file on disk). JS: `commands.removePlugin(id)`.
#[tauri::command]
#[specta::specta]
pub fn remove_plugin(app: AppHandle<Wry>, id: String) -> Result<(), String> {
    let _guard = plugins_lock().lock().unwrap_or_else(|e| e.into_inner());
    let store = plugins_path(&app)?;
    let mut plugins = load_from(&store)?;
    remove_from(&mut plugins, &id)?;
    save_to(&store, &plugins)
}

/// Load an ENABLED plugin's Tama skin (PER-47). Resolves the plugin by id,
/// requires it to be enabled, then [`build_skin`]s its declared pose assets
/// into `data:`-URI sprites — reading ONLY files strictly inside the plugin's
/// own canonicalized source dir (see [`build_skin`]/[`asset_is_within_dir`] for
/// the traversal + symlink-escape guard). File IO, so the whole load runs on
/// the blocking pool. JS: `commands.loadPluginSkin(pluginId)`.
#[tauri::command]
#[specta::specta]
pub async fn load_plugin_skin(app: AppHandle<Wry>, plugin_id: String) -> Result<TamaSkin, String> {
    let store = plugins_path(&app)?;
    crate::blocking::run_blocking(move || {
        let plugin = load_from(&store)?
            .into_iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| format!("No plugin with id {plugin_id:?} is installed."))?;
        if !plugin.enabled {
            return Err(format!("Plugin {plugin_id:?} is disabled."));
        }
        Ok(build_skin(&plugin))
    })
    .await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gitcat-plugin-registry-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ))
    }

    fn sample_plugin(id: &str) -> Plugin {
        Plugin {
            id: id.to_string(),
            name: "Sample".into(),
            version: "1.0.0".into(),
            description: Some("a test plugin".into()),
            enabled: true,
            commands: vec![PluginCommand {
                id: "greet".into(),
                label: "Greet".into(),
                run: "echo hi".into(),
                context: PluginContext::Commit,
                placement: PluginPlacement::Both,
                mutates: false,
            }],
            hooks: vec![PluginHook { event: PluginEvent::PostMutation, run: "echo done".into(), mutates: false }],
            tama: None,
            dir: None,
        }
    }

    #[test]
    fn missing_file_loads_as_empty_not_an_error() {
        let path = temp_dir("missing").join(FILE_NAME);
        assert!(!path.exists());
        let plugins = load_from(&path).expect("missing file should load empty, not error");
        assert!(plugins.is_empty());
    }

    #[test]
    fn corrupt_file_recovers_as_empty_and_backs_up_the_original_bytes() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "not json at all").unwrap();
        let plugins = load_from(&path).expect("corrupt file should recover, not hard-error");
        assert!(plugins.is_empty());
        assert!(!path.exists(), "the corrupt file must be renamed aside");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);

        let plugins = vec![sample_plugin("alpha"), sample_plugin("beta")];
        save_to(&path, &plugins).expect("save_to failed");

        let loaded = load_from(&path).expect("load_from failed");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "alpha");
        assert_eq!(loaded[0].version, "1.0.0");
        assert_eq!(loaded[0].description.as_deref(), Some("a test plugin"));
        assert!(loaded[0].enabled);
        assert_eq!(loaded[0].commands.len(), 1);
        assert_eq!(loaded[0].commands[0].id, "greet");
        assert_eq!(loaded[0].commands[0].context, PluginContext::Commit);
        assert_eq!(loaded[0].commands[0].placement, PluginPlacement::Both);
        assert_eq!(loaded[0].hooks.len(), 1);
        assert_eq!(loaded[0].hooks[0].event, PluginEvent::PostMutation);
        assert_eq!(loaded[1].id, "beta");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_omitting_optionals_defaults_enabled_true_and_empty_lists() {
        // A minimal manifest: no `enabled`, no `commands`/`hooks`, and a command
        // with no context/placement. enabled must default true, the lists empty,
        // and the command's context/placement to their documented defaults.
        let dir = temp_dir("defaults");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"version":1,"plugins":[{"id":"mini","name":"Mini","version":"0.1.0",
                "commands":[{"id":"run","label":"Run","run":"echo hi"}]}]}"#,
        )
        .unwrap();

        let loaded = load_from(&path).expect("minimal manifest must deserialize");
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].enabled, "omitted `enabled` must default to true");
        assert!(loaded[0].hooks.is_empty(), "omitted `hooks` must default to empty");
        assert_eq!(loaded[0].commands[0].context, PluginContext::None, "omitted context => None");
        assert_eq!(loaded[0].commands[0].placement, PluginPlacement::Palette, "omitted placement => Palette");
        assert!(!loaded[0].commands[0].mutates, "omitted `mutates` must default to false (read-only)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mutates_defaults_false_and_v1_manifest_without_it_still_loads() {
        // A pre-`mutates` (v1) manifest — a command AND a hook, neither declaring
        // `mutates` — must still deserialize, with both defaulting to false
        // (backward compatibility: the field is `#[serde(default)]`). A manifest
        // that DOES declare `mutates: true` must round-trip that flag.
        let dir = temp_dir("mutates");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"version":1,"plugins":[
                {"id":"v1","name":"V1","version":"1.0.0",
                 "commands":[{"id":"read","label":"Read","run":"git status"},
                             {"id":"reset","label":"Reset","run":"git reset --hard","mutates":true}],
                 "hooks":[{"event":"post-mutation","run":"echo obs"},
                          {"event":"pre-mutation","run":"git stash","mutates":true}]}
            ]}"#,
        )
        .unwrap();

        let loaded = load_from(&path).expect("a v1 manifest with no `mutates` must still load");
        assert_eq!(loaded.len(), 1);
        // Command: omitted => false; declared => true (round-trips).
        assert!(!loaded[0].commands[0].mutates, "a command omitting `mutates` defaults to false");
        assert!(loaded[0].commands[1].mutates, "`mutates: true` must round-trip on a command");
        // Hook: omitted => false; declared => true (round-trips).
        assert!(!loaded[0].hooks[0].mutates, "a hook omitting `mutates` defaults to false");
        assert!(loaded[0].hooks[1].mutates, "`mutates: true` must round-trip on a hook");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn id_charset_validation() {
        // Valid: lowercase letters/digits/'-' with a letter-or-digit first char.
        // Per ^[a-z0-9][a-z0-9-]*$ a TRAILING dash is allowed (only the FIRST
        // char is constrained to a letter/digit).
        for good in ["a", "1", "my-plugin", "gitcat-lint", "x9", "0abc-def", "trail-"] {
            let p = sample_plugin(good);
            assert!(validate_manifest(&p).is_ok(), "{good:?} should be a valid id");
        }
        // Invalid: uppercase, leading '-', spaces, underscores, dots, non-ASCII, empty.
        for bad in ["", "-lead", "Upper", "has space", "under_score", "dot.id", "café"] {
            let mut p = sample_plugin("placeholder");
            p.id = bad.to_string();
            let err = validate_manifest(&p).unwrap_err();
            assert!(err.contains("invalid"), "{bad:?} should be rejected, got: {err}");
        }
    }

    #[test]
    fn empty_name_version_or_run_is_rejected() {
        let mut p = sample_plugin("ok");
        p.name = "   ".into();
        assert!(validate_manifest(&p).unwrap_err().contains("name"));

        let mut p = sample_plugin("ok");
        p.version = "".into();
        assert!(validate_manifest(&p).unwrap_err().contains("version"));

        let mut p = sample_plugin("ok");
        p.commands[0].run = "  ".into();
        assert!(validate_manifest(&p).unwrap_err().contains("run"));
    }

    #[test]
    fn install_from_file_then_duplicate_id_is_rejected() {
        let dir = temp_dir("install");
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join(FILE_NAME);
        let manifest = dir.join("plugin.json");
        std::fs::write(
            &manifest,
            r#"{"id":"lint","name":"Lint","version":"1.0.0","commands":[{"id":"run","label":"Run","run":"echo hi"}]}"#,
        )
        .unwrap();

        let installed = install_from(&store, &manifest).expect("first install should succeed");
        assert_eq!(installed.id, "lint");
        assert!(installed.enabled, "a freshly installed plugin defaults enabled");
        assert_eq!(load_from(&store).unwrap().len(), 1);

        // Same id again => rejected, and the registry is unchanged.
        let err = install_from(&store, &manifest).expect_err("a duplicate id must be rejected");
        assert!(err.contains("already installed"), "got: {err}");
        assert_eq!(load_from(&store).unwrap().len(), 1, "a rejected install must not append");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_from_directory_finds_plugin_json() {
        let dir = temp_dir("install-dir");
        let plugin_dir = dir.join("my-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let store = dir.join(FILE_NAME);
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"frompath","name":"FromPath","version":"2.0.0"}"#,
        )
        .unwrap();

        // Pass the DIRECTORY, not the file — install must locate plugin.json in it.
        let installed = install_from(&store, &plugin_dir).expect("installing from a dir should work");
        assert_eq!(installed.id, "frompath");
        assert_eq!(installed.version, "2.0.0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversize_manifest_is_rejected() {
        let dir = temp_dir("oversize");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("plugin.json");
        // A valid JSON envelope padded past the 256KB cap with a huge description.
        let big = "x".repeat((MAX_MANIFEST_BYTES as usize) + 1);
        std::fs::write(&manifest, format!(r#"{{"id":"big","name":"Big","version":"1.0.0","description":"{big}"}}"#))
            .unwrap();
        let err = read_and_validate_manifest(&manifest).expect_err("an oversize manifest must be rejected");
        assert!(err.contains("too large"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shipped_example_plugins_are_valid_and_uniquely_ided() {
        // The reference plugins under examples/plugins/ (PER-49) must stay valid
        // as the manifest schema evolves. This smoke test walks every
        // examples/plugins/<dir>/plugin.json, reads + validates it through the
        // module's OWN read_and_validate_manifest (so it exercises the real
        // schema, size cap, and validation path), and asserts their ids are
        // unique across the examples. CARGO_MANIFEST_DIR is `src-tauri/`, so the
        // examples live one level up at `../examples/plugins`.
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/plugins");
        let entries = std::fs::read_dir(&examples)
            .unwrap_or_else(|e| panic!("could not read examples dir {}: {e}", examples.display()));

        let mut ids: Vec<String> = Vec::new();
        for entry in entries {
            let entry = entry.expect("read_dir entry");
            let dir = entry.path();
            if !dir.is_dir() {
                continue; // skip the top-level README.md and any stray file
            }
            let manifest = dir.join("plugin.json");
            if !manifest.is_file() {
                continue; // a subdir without a manifest isn't an example to validate
            }
            let plugin = read_and_validate_manifest(&manifest)
                .unwrap_or_else(|e| panic!("example manifest {} failed to validate: {e}", manifest.display()));
            ids.push(plugin.id);
        }

        assert!(ids.len() >= 2, "expected the shipped example plugins, found {}: {ids:?}", ids.len());
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "example plugin ids must be unique across examples: {ids:?}");
    }

    #[test]
    fn enable_toggle() {
        let mut plugins = vec![sample_plugin("alpha"), sample_plugin("beta")];
        set_enabled_in(&mut plugins, "beta", false).expect("toggling an existing plugin should succeed");
        assert!(plugins[0].enabled);
        assert!(!plugins[1].enabled);
        set_enabled_in(&mut plugins, "beta", true).unwrap();
        assert!(plugins[1].enabled);

        let err = set_enabled_in(&mut plugins, "ghost", false).unwrap_err();
        assert!(err.contains("No plugin with id"), "got: {err}");
    }

    #[test]
    fn remove() {
        let mut plugins = vec![sample_plugin("alpha"), sample_plugin("beta")];
        remove_from(&mut plugins, "alpha").expect("removing an existing plugin should succeed");
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "beta");

        let err = remove_from(&mut plugins, "alpha").unwrap_err();
        assert!(err.contains("No plugin with id"), "removing a missing id should error, got: {err}");
    }

    #[test]
    fn concurrent_writers_never_lose_a_write() {
        // Mirrors repo_registry.rs's own concurrency regression test: several
        // threads each append a distinct plugin under the shared lock; every
        // entry must survive (no unlocked "last write wins" drop).
        let dir = temp_dir("concurrent");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);

        const WRITERS: usize = 12;
        std::thread::scope(|scope| {
            for i in 0..WRITERS {
                let path = &path;
                scope.spawn(move || {
                    let _guard = plugins_lock().lock().unwrap_or_else(|e| e.into_inner());
                    let mut plugins = load_from(path).expect("load under lock should succeed");
                    plugins.push(sample_plugin(&format!("plugin{i}")));
                    save_to(path, &plugins).expect("save under lock should succeed");
                });
            }
        });

        let plugins = load_from(&path).expect("final load should succeed");
        assert_eq!(plugins.len(), WRITERS, "every concurrent writer's entry must survive");
        for i in 0..WRITERS {
            assert!(plugins.iter().any(|p| p.id == format!("plugin{i}")), "missing entry from writer {i}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_command_addresses_by_plugin_and_command_id() {
        // Pure lookup logic, exercised directly against a plugin list (find_command
        // itself needs an AppHandle; this mirrors its into_iter().find(...) body).
        let plugins = vec![sample_plugin("alpha")];
        let found = plugins
            .iter()
            .find(|p| p.id == "alpha")
            .and_then(|p| p.commands.iter().find(|c| c.id == "greet"));
        assert!(found.is_some());
        let missing = plugins
            .iter()
            .find(|p| p.id == "alpha")
            .and_then(|p| p.commands.iter().find(|c| c.id == "nope"));
        assert!(missing.is_none());
    }

    // -- Tama skin (PER-47) --------------------------------------------------

    #[test]
    fn manifest_with_a_valid_tama_parses_validates_and_round_trips() {
        // A manifest declaring a Tama skin (poses + copy) must deserialize, pass
        // validation, and round-trip through save/load with its skin intact.
        let dir = temp_dir("tama-valid");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"version":1,"plugins":[{"id":"skin","name":"Skin","version":"1.0.0",
                "tama":{"poses":{"curious":"assets/curious.webp","happy":"assets/happy.png"},
                        "copy":{"curious":"hmm?"}}}]}"#,
        )
        .unwrap();

        let loaded = load_from(&path).expect("a manifest with a tama skin must deserialize");
        assert_eq!(loaded.len(), 1);
        let tama = loaded[0].tama.as_ref().expect("tama must be present");
        assert_eq!(tama.poses.get("curious").map(String::as_str), Some("assets/curious.webp"));
        assert_eq!(tama.poses.get("happy").map(String::as_str), Some("assets/happy.png"));
        assert_eq!(tama.copy.get("curious").map(String::as_str), Some("hmm?"));
        validate_manifest(&loaded[0]).expect("a valid tama must pass validation");

        // Round-trip: save then reload preserves the skin.
        save_to(&path, &loaded).unwrap();
        let again = load_from(&path).expect("round-trip load");
        assert_eq!(again[0].tama.as_ref().unwrap().poses.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tama_manifest_omitting_copy_defaults_it_empty() {
        let dir = temp_dir("tama-nocopy");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"version":1,"plugins":[{"id":"skin","name":"Skin","version":"1.0.0",
                "tama":{"poses":{"hero":"hero.webp"}}}]}"#,
        )
        .unwrap();
        let loaded = load_from(&path).expect("tama without copy must still load");
        assert!(loaded[0].tama.as_ref().unwrap().copy.is_empty(), "omitted copy defaults empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tama_with_an_unknown_pose_key_is_rejected() {
        let mut p = sample_plugin("skin");
        let mut poses = HashMap::new();
        poses.insert("wiggle".to_string(), "assets/wiggle.webp".to_string());
        p.tama = Some(PluginTama { poses, copy: HashMap::new() });
        let err = validate_manifest(&p).unwrap_err();
        assert!(err.contains("wiggle") && err.contains("built-in pose"), "got: {err}");
    }

    #[test]
    fn every_built_in_pose_key_is_accepted() {
        // All eight documented keys must validate (guards against a typo in the
        // VALID_TAMA_POSE_KEYS list drifting from the contract).
        for key in VALID_TAMA_POSE_KEYS {
            let mut p = sample_plugin("skin");
            let mut poses = HashMap::new();
            poses.insert(key.to_string(), format!("assets/{key}.webp"));
            p.tama = Some(PluginTama { poses, copy: HashMap::new() });
            validate_manifest(&p).unwrap_or_else(|e| panic!("pose key {key:?} should be valid: {e}"));
        }
    }

    #[test]
    fn tama_pose_path_with_dotdot_or_absolute_is_rejected_at_validation() {
        // Parent-escaping: contains "..".
        let mut p = sample_plugin("skin");
        let mut poses = HashMap::new();
        poses.insert("curious".to_string(), "../../etc/passwd".to_string());
        p.tama = Some(PluginTama { poses, copy: HashMap::new() });
        let err = validate_manifest(&p).unwrap_err();
        assert!(err.contains("unsafe asset path"), "dotdot path should be rejected, got: {err}");

        // Absolute path.
        let mut p = sample_plugin("skin");
        let mut poses = HashMap::new();
        #[cfg(unix)]
        poses.insert("curious".to_string(), "/etc/passwd".to_string());
        #[cfg(windows)]
        poses.insert("curious".to_string(), r"C:\Windows\system32\x.webp".to_string());
        p.tama = Some(PluginTama { poses, copy: HashMap::new() });
        let err = validate_manifest(&p).unwrap_err();
        assert!(err.contains("unsafe asset path"), "absolute path should be rejected, got: {err}");
    }

    #[test]
    fn asset_containment_guard_accepts_inside_and_rejects_outside() {
        // The pure, SECURITY-CRITICAL path-containment check the loader relies
        // on (both args are treated as already-canonicalized).
        let base = temp_dir("containment");
        let plugin_dir = base.join("plugin");
        std::fs::create_dir_all(plugin_dir.join("assets")).unwrap();
        let canon_dir = std::fs::canonicalize(&plugin_dir).unwrap();

        // Genuinely inside.
        let inside = canon_dir.join("assets").join("curious.webp");
        assert!(asset_is_within_dir(&canon_dir, &inside), "an asset under the dir is contained");

        // A sibling directory sharing a prefix must NOT be treated as inside
        // (component-wise starts_with, not raw string prefix).
        let sibling = base.join("plugin-evil").join("x.webp");
        assert!(
            !asset_is_within_dir(&canon_dir, &sibling),
            "a prefix-sharing sibling dir must not be considered inside"
        );

        // A wholly unrelated absolute path (what a symlink to /etc/passwd would
        // canonicalize to) is outside.
        let outside = Path::new("/etc/passwd");
        assert!(!asset_is_within_dir(&canon_dir, outside), "an unrelated path is outside");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_skin_enforces_extension_and_size_and_encodes_a_data_uri() {
        let base = temp_dir("build-skin");
        let plugin_dir = base.join("plugin");
        let assets = plugin_dir.join("assets");
        std::fs::create_dir_all(&assets).unwrap();

        // A small valid-extension webp and a png (contents need not be real
        // images — build_skin just reads bytes and base64-encodes them).
        std::fs::write(assets.join("curious.webp"), b"RIFF-fake-webp-bytes").unwrap();
        std::fs::write(assets.join("happy.png"), b"\x89PNG-fake-png-bytes").unwrap();
        // A disallowed extension (.gif) — must be skipped.
        std::fs::write(assets.join("hero.gif"), b"GIF89a").unwrap();
        // An oversize webp (> MAX_TAMA_ASSET_BYTES) — must be skipped.
        std::fs::write(assets.join("thinking.webp"), vec![0u8; (MAX_TAMA_ASSET_BYTES as usize) + 1]).unwrap();
        // A declared pose whose file does not exist — must be skipped.

        let mut plugin = sample_plugin("skin");
        plugin.dir = Some(std::fs::canonicalize(&plugin_dir).unwrap().to_string_lossy().into_owned());
        let mut poses = HashMap::new();
        poses.insert("curious".to_string(), "assets/curious.webp".to_string());
        poses.insert("happy".to_string(), "assets/happy.png".to_string());
        poses.insert("hero".to_string(), "assets/hero.gif".to_string()); // bad ext
        poses.insert("thinking".to_string(), "assets/thinking.webp".to_string()); // oversize
        poses.insert("sleep".to_string(), "assets/missing.webp".to_string()); // missing
        let mut copy = HashMap::new();
        copy.insert("curious".to_string(), "hmm?".to_string());
        plugin.tama = Some(PluginTama { poses, copy });

        let skin = build_skin(&plugin);

        // Only the two valid, in-size, allow-listed assets survive.
        assert_eq!(skin.poses.len(), 2, "only valid+in-size+allow-listed assets survive: {:?}", skin.poses.keys().collect::<Vec<_>>());
        assert!(skin.poses.get("curious").unwrap().starts_with("data:image/webp;base64,"));
        assert!(skin.poses.get("happy").unwrap().starts_with("data:image/png;base64,"));
        assert!(!skin.poses.contains_key("hero"), "a disallowed extension must be skipped");
        assert!(!skin.poses.contains_key("thinking"), "an oversize asset must be skipped");
        assert!(!skin.poses.contains_key("sleep"), "a missing asset must be skipped");
        // copy passes through verbatim.
        assert_eq!(skin.copy.get("curious").map(String::as_str), Some("hmm?"));

        // Verify a data URI actually decodes back to the original bytes.
        use base64::Engine;
        let uri = skin.poses.get("curious").unwrap();
        let b64 = uri.strip_prefix("data:image/webp;base64,").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        assert_eq!(decoded, b"RIFF-fake-webp-bytes");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_skin_without_tama_or_dir_yields_empty_or_copy_only() {
        // No tama at all => wholly empty skin.
        let plain = sample_plugin("plain");
        let skin = build_skin(&plain);
        assert!(skin.poses.is_empty() && skin.copy.is_empty());

        // tama present but no resolvable dir => copy passes through, poses empty
        // (documented: assets unavailable without a source dir).
        let mut p = sample_plugin("nodir");
        let mut poses = HashMap::new();
        poses.insert("curious".to_string(), "assets/curious.webp".to_string());
        let mut copy = HashMap::new();
        copy.insert("curious".to_string(), "hmm?".to_string());
        p.tama = Some(PluginTama { poses, copy });
        p.dir = None;
        let skin = build_skin(&p);
        assert!(skin.poses.is_empty(), "no dir => no assets");
        assert_eq!(skin.copy.get("curious").map(String::as_str), Some("hmm?"), "copy still passes through");
    }

    #[cfg(unix)]
    #[test]
    fn build_skin_rejects_a_symlink_escaping_the_plugin_dir() {
        // The traversal-string check can't catch this: a symlink INSIDE the
        // plugin dir pointing at a file OUTSIDE it. Only the canonicalizing
        // containment guard rejects it — the load must read NOTHING.
        let base = temp_dir("symlink-escape");
        let plugin_dir = base.join("plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        // A secret file OUTSIDE the plugin dir.
        let secret = base.join("secret.webp");
        std::fs::write(&secret, b"top-secret-bytes").unwrap();
        // A symlink INSIDE the plugin dir pointing at the outside secret.
        let link = plugin_dir.join("escape.webp");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let mut plugin = sample_plugin("evil");
        plugin.dir = Some(std::fs::canonicalize(&plugin_dir).unwrap().to_string_lossy().into_owned());
        let mut poses = HashMap::new();
        poses.insert("curious".to_string(), "escape.webp".to_string());
        plugin.tama = Some(PluginTama { poses, copy: HashMap::new() });

        let skin = build_skin(&plugin);
        assert!(skin.poses.is_empty(), "a symlink escaping the plugin dir must be skipped, got: {:?}", skin.poses);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn read_and_validate_manifest_captures_the_canonical_source_dir() {
        // install must record the plugin's SOURCE dir (canonical parent of
        // plugin.json) so a later skin load can resolve relative assets.
        let dir = temp_dir("capture-dir");
        let plugin_dir = dir.join("my-skin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"id":"skin","name":"Skin","version":"1.0.0","tama":{"poses":{"hero":"hero.webp"}}}"#,
        )
        .unwrap();

        let plugin = read_and_validate_manifest(&plugin_dir).expect("must validate");
        let recorded = plugin.dir.expect("dir must be captured");
        let expected = std::fs::canonicalize(&plugin_dir).unwrap().to_string_lossy().into_owned();
        assert_eq!(recorded, expected, "dir must be the canonicalized plugin directory");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// INTEGRATION (apply by hand in src-tauri/src/lib.rs — do NOT auto-edit)
// ---------------------------------------------------------------------------
//
// 1. Module declaration — add near the other `pub mod` lines (e.g. right after
//    `pub mod pickaxe;` / before `pub mod plumbing;`, keeping the rough
//    alphabetical grouping):
//
//        pub mod plugin_registry; // PER-39: app-level plugin registry (plugins.json under app_config_dir) + install/enable/remove CRUD
//
// 2. Commands — add these four paths inside the `collect_commands![ ... ]`
//    macro in `specta_builder()` (anywhere in the list; grouping them near the
//    other app-level-JSON commands like `tool_settings::*` reads best):
//
//        plugin_registry::list_plugins,
//        plugin_registry::set_plugin_enabled,
//        plugin_registry::install_plugin_from_path,
//        plugin_registry::remove_plugin,
//        plugin_registry::load_plugin_skin, // PER-47: load a plugin's Tama skin (pose data: URIs + copy)
//
//    There is NO separate invoke_handler command list to update: the app's
//    `.invoke_handler(builder.invoke_handler())` derives entirely from
//    `collect_commands!`, so adding the four lines above is sufficient. (Regen
//    the TS bindings via `cargo test export_bindings`, same as any new command.)
