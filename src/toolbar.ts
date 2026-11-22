// Which tools the bar shows and in what order (Settings → Toolbar). Collapse, settings and the grip stay fixed.
import { icons } from "./icons";

export type ToolId = "mic" | "speaker" | "pen" | "bluetooth" | "media" | "camera" | "clipboard" | "files";
export const TOOLS: [ToolId, string, keyof typeof icons][] = [
  ["mic", "Microphone", "mic"],
  ["speaker", "Speaker", "speaker"],
  ["pen", "Pen", "pen"],
  ["bluetooth", "Bluetooth batteries", "bluetooth"],
  ["media", "Media", "playPause"],
  ["camera", "Meet camera", "camera"],
  ["clipboard", "Clipboard sync", "clipboard"],
  ["files", "Send files", "files"],
];
export interface ToolbarLayout { order: ToolId[]; hidden: ToolId[] }

const isTool = (v: unknown): v is ToolId => TOOLS.some(([id]) => id === v);

/** Stored layout → a full one: unknown ids dropped, tools added in a later version appended (shown). */
export function parseToolbar(raw: string): ToolbarLayout {
  let v: { order?: unknown; hidden?: unknown } = {};
  try {
    v = JSON.parse(raw || "{}");
  } catch {
    /* corrupt: fall back to the default layout */
  }
  const saved = Array.isArray(v.order) ? v.order.filter(isTool) : [];
  const order = [...new Set([...saved, ...TOOLS.map(([id]) => id)])];
  const hidden = Array.isArray(v.hidden) ? [...new Set(v.hidden.filter(isTool))] : [];
  return { order, hidden };
}

export function toolbarTab(t: ToolbarLayout): string {
  const rows = t.order.map((id) => {
    const [, label, icon] = TOOLS.find(([tid]) => tid === id)!;
    const on = !t.hidden.includes(id);
    return `<div class="tool-row" data-tool-row="${id}">
      <span class="drag" data-drag title="Drag to reorder" aria-hidden="true">${icons.grip}</span>
      <button class="row toggle" role="switch" aria-checked="${on}" data-toolbar="${id}" title="Alt+↑/↓ moves it">
        <span class="ico-sm">${icons[icon]}</span><span class="name">${label}</span><span class="knob"></span></button></div>`;
  }).join("");
  return `<p class="hint">Drag ⠿ to reorder. Switch off what you don't need.</p><div class="list tools">${rows}</div>`;
}
