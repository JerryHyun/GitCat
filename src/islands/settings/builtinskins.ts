// Built-in Tama characters (PER-53).
//
// GitCat ships two extra painted characters — Momo (pink) and Sora (blue) —
// alongside her default portraits. Unlike a PLUGIN skin (which the Rust side
// loads from an on-disk manifest via commands.loadPluginSkin), a built-in is
// pure frontend: its 8 recoloured poses are real image assets bundled at build
// time. Each `import` of a `.webp` below is turned by Vite into a hashed asset
// URL string (see `vite/client`'s own module typing), so `poses` is a plain
// poseKey -> URL map ready to hand straight to bridge.applyTamaSkin — no backend
// round-trip, which is also why built-ins keep working in design mode (!IN_TAURI)
// where there's no plugin backend at all.
//
// Each character also carries a `voicePitch` multiplier (see legacy/sound.ts):
// Momo speaks a touch higher, Sora a touch lower, so switching characters shifts
// her voice too, not just her look. `copy` is an optional greeting line the
// picker surfaces once when the character is applied (same courtesy toast a
// plugin skin's copy gets — see pickSkinCopyLine).

// ── Momo (pink) ──
import momoHero from "../../assets/skins/momo/hero.webp";
import momoCurious from "../../assets/skins/momo/curious.webp";
import momoConfident from "../../assets/skins/momo/confident.webp";
import momoThinking from "../../assets/skins/momo/thinking.webp";
import momoHappy from "../../assets/skins/momo/happy.webp";
import momoAlarm from "../../assets/skins/momo/alarm.webp";
import momoShocked from "../../assets/skins/momo/shocked.webp";
import momoSleep from "../../assets/skins/momo/sleep.webp";

// ── Sora (blue) ──
import soraHero from "../../assets/skins/sora/hero.webp";
import soraCurious from "../../assets/skins/sora/curious.webp";
import soraConfident from "../../assets/skins/sora/confident.webp";
import soraThinking from "../../assets/skins/sora/thinking.webp";
import soraHappy from "../../assets/skins/sora/happy.webp";
import soraAlarm from "../../assets/skins/sora/alarm.webp";
import soraShocked from "../../assets/skins/sora/shocked.webp";
import soraSleep from "../../assets/skins/sora/sleep.webp";

// The 8 painted poses every Tama look provides — the same keys as legacy/main.ts's
// TAMA_IMG. A skin maps each to an image URL; any pose it omits falls back to the
// built-in painted art (see tamaPose), but the built-in characters ship all 8.
export type TamaPoseKey = "hero" | "curious" | "confident" | "thinking" | "happy" | "alarm" | "shocked" | "sleep";

export interface BuiltinSkin {
  // The "builtin:*" prefix keeps a built-in id distinct from any plugin id in
  // the shared tamaSkinPluginId persistence (a plugin id can never start with
  // "builtin:" — plugin ids match ^[a-z0-9][a-z0-9-]*$, no colon), so the picker
  // and the boot-apply path can tell the two sources apart from the id alone.
  id: "builtin:momo" | "builtin:sora";
  name: string;
  poses: Record<TamaPoseKey, string>;
  // Multiplier fed to sound.ts's setVoicePitch (clamped there to [0.5, 2.0]).
  voicePitch: number;
  // Optional greeting line, surfaced once when the character is applied.
  copy?: Record<string, string>;
}

export const BUILTIN_SKINS: BuiltinSkin[] = [
  {
    id: "builtin:momo",
    name: "Momo (pink)",
    poses: {
      hero: momoHero,
      curious: momoCurious,
      confident: momoConfident,
      thinking: momoThinking,
      happy: momoHappy,
      alarm: momoAlarm,
      shocked: momoShocked,
      sleep: momoSleep,
    },
    voicePitch: 1.12,
    copy: { greeting: "Momo here — let's keep your history safe! ♪" },
  },
  {
    id: "builtin:sora",
    name: "Sora (blue)",
    poses: {
      hero: soraHero,
      curious: soraCurious,
      confident: soraConfident,
      thinking: soraThinking,
      happy: soraHappy,
      alarm: soraAlarm,
      shocked: soraShocked,
      sleep: soraSleep,
    },
    voicePitch: 0.9,
    copy: { greeting: "Sora, at your service. Steady hands on the repo." },
  },
];

// Fast lookup by id for the picker's setTamaSkin / the boot-apply path. A built-in
// id that isn't found (e.g. a persisted id from a future/older build) returns
// undefined and the caller falls back to Default — the same silent fail-safe the
// plugin path uses for a removed plugin.
export function builtinSkinById(id: string | null | undefined): BuiltinSkin | undefined {
  if (!id) return undefined;
  return BUILTIN_SKINS.find((s) => s.id === id);
}

// Whether a persisted/selected id names a built-in character (vs a plugin id or
// Default). Cheap prefix check — see BuiltinSkin.id's own note on why the prefix
// is unambiguous.
export function isBuiltinSkinId(id: string | null | undefined): boolean {
  return typeof id === "string" && id.startsWith("builtin:");
}
