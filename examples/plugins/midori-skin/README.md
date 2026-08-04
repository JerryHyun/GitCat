# Midori (green)

A **full-character Tama skin**: a plugin that reskins Tama end to end — all
eight poses plus a subtly different voice — instead of adding a command or hook.
Midori is a green recolor with a slightly brighter (higher-pitched) voice.

## What it shows

- The **`tama` manifest field**. A plugin ships an alternate look for Tama by
  declaring a `tama` object. It contributes no commands and no hooks — a skin is
  purely declarative art + copy.
- **`poses`** — a map from each of Tama's eight built-in pose keys to a
  **relative** image path inside the plugin folder:

  | Pose key | When Tama wears it |
  | --------- | ------------------ |
  | `hero` | greeting / empty-state hero |
  | `curious` | idle, and a freshly opened repo |
  | `confident` | after a rescue/undo |
  | `thinking` | long-running / syncing ops |
  | `happy` | celebrate |
  | `alarm` | a flagged destructive action |
  | `shocked` | caution / a failed op |
  | `sleep` | mouse-idle nap |

  A skin may override **some or all** of the eight — any key it omits falls back
  to Tama's default painted portrait. Midori provides all eight, so it's a
  complete character.
- **`voicePitch`** — a multiplier (here `1.05`) applied to Tama's synthesized
  sound effects, so Midori speaks a touch brighter than the default. Omit it for
  "no change" (the default `1.0`). GitCat clamps it to a sane `[0.5, 2.0]` range
  when the skin loads; a non-finite value is rejected at install time.
- **`copy`** — optional greeting/voice lines. GitCat surfaces one (preferring
  `applied` > `greeting` > `hero`) as a courtesy toast when the skin is applied.
  It never reaches a safety-critical pose — same trust level as the
  `::gitcat.tama` reaction protocol.

## Manifest shape

```jsonc
{
  "id": "midori-skin",
  "name": "Midori (green)",
  "version": "1.0.0",
  "tama": {
    "poses": {
      "hero": "poses/hero.webp",
      "curious": "poses/curious.webp"
      // …the remaining six keys…
    },
    "voicePitch": 1.05,          // optional; omit for the default 1.0
    "copy": { "greeting": "…" }  // optional
  }
}
```

Pose paths are **relative to the plugin folder** and may not escape it (`..` or
absolute paths are refused at install time), the same containment guard every
plugin asset gets.

## Install

Settings → **Plugins** → **Install plugin…** and pick this folder's
`plugin.json` (or the folder itself). Then open Settings → **Tama** → **Skin**
and choose **Midori (green)** from the picker. Tama's portraits and voice swap
live; pick **Default (built-in)** to switch back.

## Built-in characters

GitCat also ships two characters with no plugin needed — **Momo (pink)** and
**Sora (blue)** — always available in the same Skin picker. Midori is the plugin
counterpart: the same feature, contributed from outside the app.

## Note

A skin contributes **no commands and no hooks**, so it runs no external process
and needs no open repository — it only changes how Tama looks and sounds.
