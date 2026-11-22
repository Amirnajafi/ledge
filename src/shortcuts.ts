// Global shortcuts: one per dock action, recorded in Settings, stored per device in localStorage.
import { register, unregisterAll } from "@tauri-apps/plugin-global-shortcut";

export const ACTIONS = [
  { id: "mic", label: "Mic mute / unmute" },
  { id: "speaker", label: "Speaker mute / unmute" },
  { id: "play", label: "Play / pause" },
  { id: "next", label: "Next track" },
  { id: "prev", label: "Previous track" },
  { id: "meet", label: "Meet camera on / off" },
  { id: "mirror", label: "Show / hide camera mirror" },
  { id: "mouse", label: "Magic Mouse taps on / off" },
  { id: "draw", label: "Pen: draw on screen" },
] as const;

export type ActionId = (typeof ACTIONS)[number]["id"];
export type Shortcuts = Partial<Record<ActionId, string>>;

const MODIFIERS: [keyof KeyboardEvent, string, string][] = [
  ["ctrlKey", "Control", "⌃"],
  ["altKey", "Alt", "⌥"],
  ["shiftKey", "Shift", "⇧"],
  ["metaKey", "Command", "⌘"],
];

/** Accelerator like "Command+Shift+KeyM" (physical key codes, so it works on any keyboard layout).
 *  null = not a usable shortcut yet (modifier alone, or a plain key without a modifier). */
export function accelerator(e: KeyboardEvent): string | null {
  if (["Meta", "Control", "Alt", "Shift", "CapsLock", "Fn"].includes(e.key)) return null;
  const mods = MODIFIERS.filter(([k]) => e[k]).map(([, name]) => name);
  const fKey = /^F\d{1,2}$/.test(e.code);
  if (!mods.length && !fKey) return null; // a bare letter would hijack typing everywhere
  return [...mods, e.code].join("+");
}

/** "Command+Shift+KeyM" → "⇧⌘M" */
export function pretty(accel: string): string {
  const parts = accel.split("+");
  const key = parts.pop() ?? "";
  const mods = MODIFIERS.filter(([, name]) => parts.includes(name)).map(([, , sym]) => sym).join("");
  const name = key.replace(/^Key|^Digit/, "").replace(/^Arrow/, "").replace("Space", "Space ");
  return mods + name.trim();
}

/** Replace every registration with `map`. Returns the actions macOS refused (key combo taken by another app). */
export async function applyShortcuts(map: Shortcuts, run: (id: ActionId) => void): Promise<ActionId[]> {
  await unregisterAll();
  const failed: ActionId[] = [];
  for (const { id } of ACTIONS) {
    const accel = map[id];
    if (!accel) continue;
    try {
      await register(accel, (ev) => ev.state === "Pressed" && run(id));
    } catch (e) {
      console.error("[shortcuts] register failed", accel, e);
      failed.push(id);
    }
  }
  return failed;
}

export function pauseShortcuts(): Promise<void> {
  return unregisterAll();
}
