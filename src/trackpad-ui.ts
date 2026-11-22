// Settings → Trackpad: a drawn trackpad whose 4 edges (slide) and 4 corners (double-tap) are each configurable.
import { icons } from "./icons";
import { ACTIONS, type ActionId } from "./shortcuts";

export type PadSlide = "off" | "volume" | "brightness" | "tracks";
export type PadSide = "left" | "right" | "top" | "bottom";
export type PadZone = PadSide | "c0" | "c1" | "c2" | "c3"; // corners: top-left, top-right, bottom-left, bottom-right
export type PadCorner = ActionId | "";
export interface PadSettings { left: PadSlide; right: PadSlide; top: PadSlide; bottom: PadSlide; corners: PadCorner[] }

export const PAD_DEFAULTS: PadSettings = { left: "off", right: "off", top: "off", bottom: "off", corners: ["", "", "", ""] };

const SIDES: PadSide[] = ["left", "right", "top", "bottom"];
const CORNERS = ["c0", "c1", "c2", "c3"] as const;
const CORNER_NAMES = ["Top-left corner", "Top-right corner", "Bottom-left corner", "Bottom-right corner"];

const SLIDES: Record<PadSlide, { label: string; icon: string }> = {
  off: { label: "Off", icon: "" },
  volume: { label: "Volume", icon: icons.speaker },
  brightness: { label: "Brightness", icon: icons.sun },
  tracks: { label: "Skip tracks", icon: icons.next },
};

const CORNER_ACTIONS: Record<ActionId, { label: string; icon: string }> = {
  speaker: { label: "Mute sound", icon: icons.speakerOff },
  mic: { label: "Mute mic", icon: icons.micOff },
  play: { label: "Play / pause", icon: icons.playPause },
  next: { label: "Next track", icon: icons.next },
  prev: { label: "Previous", icon: icons.prev },
  meet: { label: "Meet camera", icon: icons.camera },
  mirror: { label: "Mirror", icon: icons.enlarge },
  mouse: { label: "Mouse taps", icon: icons.mouse },
  draw: { label: "Pen", icon: icons.pen },
};

const isSide = (z: PadZone): z is PadSide => (SIDES as string[]).includes(z);
const cornerIndex = (z: PadZone): number => CORNERS.indexOf(z as (typeof CORNERS)[number]);

/** Accepts stored JSON, keeping only values this version knows. */
export function parsePad(raw: string): PadSettings {
  try {
    const v = JSON.parse(raw) as Partial<Record<string, unknown>>;
    const slide = (x: unknown): PadSlide => (typeof x === "string" && x in SLIDES ? (x as PadSlide) : "off");
    const corner = (x: unknown): PadCorner => (ACTIONS.some((a) => a.id === x) ? (x as ActionId) : "");
    const corners = Array.isArray(v.corners) ? v.corners : [];
    return {
      left: slide(v.left),
      right: slide(v.right),
      top: slide(v.top),
      bottom: slide(v.bottom),
      corners: [0, 1, 2, 3].map((i) => corner(corners[i])),
    };
  } catch {
    return PAD_DEFAULTS;
  }
}

/** The shape Rust's TrackpadSettings expects. */
export function padForRust(p: PadSettings) {
  return { left: p.left, right: p.right, top: p.top, bottom: p.bottom, corners: p.corners.map((c) => c !== "") };
}

/** New settings with `zone` set to `value` (a PadSlide for edges, an ActionId or "" for corners). */
export function setZone(p: PadSettings, zone: PadZone, value: string): PadSettings {
  if (isSide(zone)) return value in SLIDES ? { ...p, [zone]: value as PadSlide } : p;
  const i = cornerIndex(zone);
  const v: PadCorner = ACTIONS.some((a) => a.id === value) ? (value as ActionId) : "";
  return { ...p, corners: p.corners.map((c, j) => (j === i ? v : c)) };
}

function zoneButton(p: PadSettings, zone: PadZone, selected: PadZone): string {
  const on = isSide(zone) ? p[zone] !== "off" : p.corners[cornerIndex(zone)] !== "";
  const icon = isSide(zone) ? SLIDES[p[zone]].icon : on ? CORNER_ACTIONS[p.corners[cornerIndex(zone)] as ActionId].icon : "";
  const name = isSide(zone) ? `${zone[0].toUpperCase()}${zone.slice(1)} edge` : CORNER_NAMES[cornerIndex(zone)];
  const kind = isSide(zone) ? `pad-edge ${zone}` : `pad-corner ${zone}`;
  return `<button class="${kind}${on ? " on" : ""}" data-zone="${zone}" aria-pressed="${zone === selected}" aria-label="${name}">${icon}</button>`;
}

function editor(p: PadSettings, zone: PadZone): string {
  if (isSide(zone)) {
    const dir = zone === "left" || zone === "right" ? "slide up / down" : "slide left / right";
    const chips = (Object.keys(SLIDES) as PadSlide[])
      .map((s) => `<button class="chip" data-pad-value="${s}" aria-pressed="${p[zone] === s}">${SLIDES[s].icon}${SLIDES[s].label}</button>`)
      .join("");
    return `<p class="pad-title"><b>${zone[0].toUpperCase()}${zone.slice(1)} edge</b> · ${dir}</p><div class="chips">${chips}</div>`;
  }
  const cur = p.corners[cornerIndex(zone)];
  const chips = [`<button class="chip" data-pad-value="" aria-pressed="${cur === ""}">Off</button>`]
    .concat(
      ACTIONS.map(({ id }) => `<button class="chip" data-pad-value="${id}" aria-pressed="${cur === id}">${CORNER_ACTIONS[id].icon}${CORNER_ACTIONS[id].label}</button>`),
    )
    .join("");
  return `<p class="pad-title"><b>${CORNER_NAMES[cornerIndex(zone)]}</b> · double-tap</p><div class="chips">${chips}</div>`;
}

export function trackpadTab(p: PadSettings, zone: PadZone, pads: number | null): string {
  const zones = [...SIDES, ...CORNERS].map((z) => zoneButton(p, z, zone)).join("");
  const status = pads === null ? "" : pads ? "" : `<p class="hint">No trackpad found.</p>`;
  return `<div class="pad"><div class="pad-surface">${zones}<span class="pad-mark">Pick an edge or a corner</span></div></div>
    ${editor(p, zone)}${status}
    <p class="hint">Slides only start when you move along the edge, so moving the pointer still works.</p>`;
}
