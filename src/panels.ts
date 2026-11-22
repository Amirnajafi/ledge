// HTML for each pop-out panel. Pure functions of state → markup; main.ts wires events.
import type { AudioKind, AudioState, Battery, Edge, MouseSettings, PenStyle, Permission, SyncSnapshot, Transfer } from "./api";
import { icons } from "./icons";
import { ACTIONS, pretty, type ActionId, type Shortcuts } from "./shortcuts";
import { trackpadTab, type PadSettings, type PadZone } from "./trackpad-ui";
import { toolbarTab, type ToolbarLayout } from "./toolbar";

export type SettingsTab = "general" | "toolbar" | "shortcuts" | "mouse" | "trackpad" | "draw" | "permissions" | "about";
export const SETTINGS_TABS: [SettingsTab, string, string][] = [
  ["general", "General", icons.sliders],
  ["toolbar", "Toolbar", icons.grip],
  ["shortcuts", "Keys", icons.keyboard],
  ["mouse", "Mouse", icons.mouse],
  ["trackpad", "Trackpad", icons.trackpad],
  ["draw", "Pen", icons.pen],
  ["permissions", "Privacy", icons.shield],
  ["about", "About", icons.info],
];

export const PEN_COLORS = ["#ff3b30", "#ff9500", "#ffcc00", "#34c759", "#0a84ff", "#bf5af2", "#ffffff", "#1c1c1e"];

function penTab(p: PenStyle, drawing: boolean): string {
  const swatches = PEN_COLORS.map(
    (c) => `<button class="swatch" style="--c:${c}" data-pen-color="${c}" aria-pressed="${p.color === c}" aria-label="Color ${c}"></button>`,
  ).join("");
  const custom = PEN_COLORS.includes(p.color) ? "" : " on";
  return `<h3>Color</h3>
    <div class="swatches">${swatches}<label class="swatch custom${custom}" style="--c:${p.color}" title="Any color">
      <input type="color" value="${p.color}" data-pen-custom aria-label="Custom color"></label></div>
    <h3>Thickness</h3>
    <label class="range-row"><span class="pen-preview" style="--c:${p.color};--w:${p.width}px"></span>
      <input type="range" min="1" max="24" step="1" value="${p.width}" data-pen-width aria-label="Thickness">
      <output data-pen-width-out>${p.width} px</output></label>
    <div class="list"><button class="row toggle" role="switch" aria-checked="${p.highlighter}" data-pen-hl>
      <span class="name">Highlighter<small>Wide, see-through strokes for marking text</small></span><span class="knob"></span></button></div>
    <button class="switch" role="switch" aria-checked="${drawing}" data-action="pen"><span>${drawing ? "Drawing on screen" : "Start drawing"}</span><span class="knob"></span></button>
    <p class="hint">Right-click undoes the last stroke. Turning the pen off clears the screen. The dock stays on top so you can always switch it off.</p>`;
}

export type PanelId = "mic" | "speaker" | "bluetooth" | "media" | "camera" | "clipboard" | "files" | "settings" | "notice";

export const BATTERY_LOW = 20;
export const BATTERY_CRITICAL = 10;

/** "" | "low" | "critical" for one charge level. */
export const batteryLevel = (pct: number): string => (pct < BATTERY_CRITICAL ? "critical" : pct < BATTERY_LOW ? "low" : "");

function bluetoothPanel(list: Battery[] | null): string {
  if (!list) return `<h2>Bluetooth</h2><p class="hint">Reading batteries…</p>`;
  const rows = list
    .map((b) => `<div class="row battery ${batteryLevel(b.percent)}"><span class="name" title="${esc(b.name)}">${esc(b.name)}</span>
      <span class="gauge" style="--p:${b.percent}%"></span><b>${b.charging ? "⚡" : ""}${b.percent}%</b></div>`)
    .join("");
  return `<h2>Bluetooth <small>lowest first</small></h2>
    <div class="list">${rows || `<p class="hint row">No accessories reporting a battery</p>`}</div>`;
}

const esc = (s: string): string =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c);

function ago(at: number): string {
  const s = Math.max(0, Math.round(Date.now() / 1000 - at));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return `${Math.round(s / 3600)} h ago`;
}

function audioPanel(kind: "mic" | "speaker", audio: AudioState | null, pref: string): string {
  const side = kind === "mic" ? audio?.input : audio?.output;
  const title = kind === "mic" ? "Microphone" : "Output";
  if (!side) return `<h2>${title}</h2><p class="hint">Reading audio devices…</p>`;
  const rows = side.devices
    .map((d) => {
      const cur = d.id === side.default;
      const starred = d.name === pref;
      return `<div class="device-row"><button class="row${cur ? " current" : ""}" data-device="${d.id}">
        <span class="check">${cur ? "✓" : ""}</span><span class="name">${esc(d.name)}</span></button>
        <button class="star${starred ? " on" : ""}" data-star="${esc(d.name)}" aria-pressed="${starred}"
          title="${starred ? "Stop preferring this device" : "Always use this device when it's connected"}">${starred ? "★" : "☆"}</button></div>`;
    })
    .join("");
  return `<h2>${title}</h2>
    <button class="switch danger" role="switch" aria-checked="${side.muted}" data-action="mute">
      <span>${side.muted ? "Muted" : "Live"}</span><span class="knob"></span></button>
    <div class="list">${rows || `<p class="hint row">No devices found</p>`}</div>`;
}

function mediaPanel(): string {
  return `<h2>Now playing <small>system media keys</small></h2>
    <div class="transport">
      <button class="tool" data-media="prev" title="Previous">${icons.prev}</button>
      <button class="tool" data-media="play" title="Play / pause">${icons.playPause}</button>
      <button class="tool" data-media="next" title="Next">${icons.next}</button>
    </div>
    <p class="hint">Controls whatever app owns Now Playing — Music, Spotify, a browser tab.</p>`;
}

export function camSizeButton(big: boolean): string {
  return `<button class="cam-size" data-action="cam-size" title="${big ? "Normal size" : "Bigger preview"}" aria-label="${big ? "Normal size" : "Bigger preview"}">${big ? icons.shrink : icons.enlarge}</button>`;
}

function cameraPanel(big: boolean): string {
  return `<h2>Camera</h2>
    <div class="video"><video autoplay muted playsinline></video><p class="hint" data-cam-hint>Starting camera…</p>${camSizeButton(big)}</div>
    <div class="list" data-cams></div>`;
}

// "Unreachable" = Tailscale answered but Ledge couldn't listen, so other devices can't see this one.
function tailscaleLabel(sync: SyncSnapshot | null): string {
  if (!sync?.me) return "Tailscale not running";
  return sync.listening ? esc(sync.me) : `${esc(sync.me)} · unreachable, other devices can't see this Mac`;
}

function clipboardPanel(sync: SyncSnapshot | null): string {
  if (!sync) return `<h2>Clipboard</h2><p class="hint">Talking to Tailscale…</p>`;
  const peers = sync.peers
    .map((p) => {
      const cls = p.dock ? "dock" : p.online ? "online" : "";
      const meta = p.dock ? "syncing" : p.online ? "Ledge not running" : "offline";
      return `<div class="row"><span class="dot ${cls}"></span><span class="name">${esc(p.name)}</span><span class="meta">${meta}</span></div>`;
    })
    .join("");
  const last = sync.last
    ? `<p class="hint">${sync.last.incoming ? "From" : "Sent to"} ${esc(sync.last.peer)} · ${ago(sync.last.at)}<br>“${esc(sync.last.preview)}”</p>`
    : `<p class="hint">Copy text on any of your devices — it lands here.</p>`;
  const me = `<small>${tailscaleLabel(sync)}</small>`;
  return `<h2>Clipboard ${me}</h2>
    <button class="switch" role="switch" aria-checked="${sync.enabled}" data-action="sync">
      <span>Sync over Tailscale</span><span class="knob"></span></button>
    <div class="list">${peers || `<p class="hint row">No devices on your tailnet</p>`}</div>
    ${last}`;
}

export function bytes(n: number): string {
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) (n /= 1024), i++;
  return `${n < 10 && i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

export function transferMeta(t: Transfer): string {
  if (t.state === "failed") return t.error ?? "failed";
  const dir = t.incoming ? "from" : "to";
  if (t.state === "done") return `${t.incoming ? "Saved · " : "Sent "}${dir} ${t.peer} · ${bytes(t.total)}`;
  return `${dir} ${t.peer} · ${bytes(t.done)} / ${bytes(t.total)}`;
}

function transferRow(t: Transfer): string {
  const pct = t.total ? Math.round((t.done / t.total) * 100) : 100;
  const tag = t.incoming && t.state === "done" && t.path ? `button data-reveal="${esc(t.path)}" title="Show in Finder"` : "div";
  return `<${tag} class="row transfer ${t.state}" data-transfer="${t.id}">
      <span class="xfer-icon">${t.incoming ? "↓" : "↑"}</span>
      <span class="xfer-body"><span class="name">${esc(t.name)}</span>
        <span class="meta">${esc(transferMeta(t))}</span>
        <span class="track"><span class="fill" style="transform:scaleX(${pct / 100})"></span></span></span>
    </${tag.split(" ")[0]}>`;
}

function filesPanel(sync: SyncSnapshot | null, target: string | null, transfers: Transfer[]): string {
  const peers = (sync?.peers ?? [])
    .map((p) => {
      const ready = p.dock && p.online;
      const cur = ready && p.ip === target;
      const meta = ready ? p.os : p.online ? "Ledge not running" : "offline";
      return `<button class="row${cur ? " current" : ""}" data-peer="${esc(p.ip)}" ${ready ? "" : "disabled"}>
        <span class="dot ${ready ? "dock" : p.online ? "online" : ""}"></span><span class="name">${esc(p.name)}</span><span class="meta">${esc(meta)}</span></button>`;
    })
    .join("");
  const targetName = sync?.peers.find((p) => p.ip === target && p.dock && p.online)?.name;
  const drop = targetName
    ? `<button class="drop" data-action="browse"><span class="drop-icon">${icons.files}</span>
        <span>Drop files here or <u>browse</u></span><small>→ ${esc(targetName)} · Downloads</small></button>`
    : `<p class="hint">Pick a device running Ledge, then drop files on the dock.</p>`;
  const list = transfers.length ? `<div class="list transfers">${transfers.map(transferRow).join("")}</div>` : "";
  return `<h2>Send files <small>${tailscaleLabel(sync)}</small></h2>
    <div class="list">${peers || `<p class="hint row">No devices on your tailnet</p>`}</div>
    ${drop}${list}`;
}

const PERM_LABEL: Record<Permission["status"], string> = {
  granted: "Allowed",
  denied: "Open settings",
  "not-asked": "Allow…",
  unknown: "Open settings",
};

// Unsigned rebuilds look like a new app to macOS, so an "on" switch can belong to an older Ledge.
const permHint = (p: Permission): string =>
  p.id === "accessibility" && p.status === "denied" ? "Already on? Select Ledge, press –, then Allow again" : p.why;

function permissionsList(perms: Permission[] | null): string {
  if (!perms) return `<p class="hint">Checking permissions…</p>`;
  const rows = perms
    .map(
      (p) => `<button class="row perm ${p.status}" data-perm="${p.id}" data-status="${p.status}" title="${p.status === "granted" ? "Manage" : "Grant"} in System Settings">
        <span class="dot perm-dot"></span>
        <span class="xfer-body"><span class="name">${esc(p.name)}</span><span class="meta">${esc(permHint(p))}</span></span>
        <span class="perm-action">${PERM_LABEL[p.status]}</span></button>`,
    )
    .join("");
  return `<div class="list">${rows}</div>`;
}

function updateRow(sync: SyncSnapshot | null): string {
  const u = sync?.update;
  if (!u) return `<p class="hint">Ledge ${esc(sync?.version ?? "")} · up to date with your devices</p>`;
  return `<button class="row update" data-action="update"><span class="dot perm-dot"></span>
    <span class="xfer-body"><span class="name">Update to ${esc(u.version ?? "")}</span><span class="meta">from ${esc(u.name)} · relaunches Ledge</span></span></button>`;
}

function shortcutsList(map: Shortcuts, recording: ActionId | null): string {
  const rows = ACTIONS.map(({ id, label }) => {
    const rec = recording === id;
    const accel = map[id];
    const keys = rec ? "Press keys…" : accel ? pretty(accel) : "Record";
    return `<button class="row shortcut${rec ? " recording" : ""}" data-shortcut="${id}" aria-pressed="${rec}">
      <span class="name">${label}</span><kbd class="${accel || rec ? "" : "empty"}">${esc(keys)}</kbd></button>`;
  }).join("");
  const hint = recording ? "Esc cancels · Delete removes the shortcut" : "Works from any app. Click one to record.";
  return `<div class="list">${rows}</div><p class="hint">${hint}</p>`;
}

export const MOUSE_OPTIONS: [keyof MouseSettings, string, string][] = [
  ["tap", "Tap to click", "Two quick taps double-click"],
  ["two_finger_right", "Two-finger tap", "Right-click"],
  ["right_side_right", "Tap the right side", "Right-click with one finger"],
  ["three_finger_middle", "Three-finger tap", "Middle click: links open in a new tab"],
];

function mouseSection(m: MouseSettings, mice: number | null): string {
  const status = mice === null ? "" : mice ? "Magic Mouse connected" : "No Magic Mouse connected";
  const rows = MOUSE_OPTIONS.map(
    ([key, name, sub]) => `<button class="row toggle" role="switch" aria-checked="${m[key]}" data-mouse="${key}">
      <span class="name">${name}<small>${sub}</small></span><span class="knob"></span></button>`,
  ).join("");
  return `<p class="hint">${status}</p><div class="list">${rows}</div>`;
}

function generalTab(edge: Edge): string {
  const b = (e: Edge, label: string) => `<button data-edge="${e}" aria-pressed="${edge === e}">${label}</button>`;
  return `<h3>Dock position</h3>
    <div class="edges">${b("top", "Top")}${b("left", "Left")}<span class="screen"></span>${b("right", "Right")}${b("bottom", "Bottom")}</div>
    <h3>Your devices</h3>
    <div class="list"><button class="row" data-action="sync-settings"><span class="ico-sm">${icons.clipboard}</span>
      <span class="xfer-body"><span class="name">Sync settings to all devices</span>
      <span class="meta">Shortcuts, toolbar, trackpad, mouse, pen, starred devices, dock position</span></span></button></div>`;
}

function aboutTab(sync: SyncSnapshot | null): string {
  return `<h3>Version</h3>${updateRow(sync)}
    <p class="hint">Updates install by themselves from any of your devices running a newer Ledge.</p>
    <h3>Credits</h3><p class="hint">Trackpad edge gestures adapted from SlideSense (MIT).</p>`;
}

function settingsPanel(d: PanelData): string {
  const tabs = SETTINGS_TABS.map(([id, label, icon]) => {
    const badge = id === "about" && d.sync?.update ? `<span class="tab-badge"></span>` : "";
    return `<button role="tab" data-tab="${id}" aria-selected="${d.tab === id}">${icon}<span>${label}</span>${badge}</button>`;
  }).join("");
  const body: Record<SettingsTab, () => string> = {
    general: () => generalTab(d.edge),
    toolbar: () => toolbarTab(d.toolbar),
    shortcuts: () => shortcutsList(d.shortcuts, d.recording),
    mouse: () => mouseSection(d.mouse, d.mice),
    trackpad: () => trackpadTab(d.trackpad, d.padZone, d.pads),
    draw: () => penTab(d.pen, d.drawing),
    permissions: () => permissionsList(d.perms),
    about: () => aboutTab(d.sync),
  };
  return `<h2>Ledge <small>${esc(d.sync?.version ?? "")}</small><button class="quit-top" data-action="quit" title="Quit Ledge" aria-label="Quit Ledge">${icons.power}</button></h2>
    <div class="tabs" role="tablist">${tabs}</div>
    <div class="tab-body" role="tabpanel">${body[d.tab]()}</div>`;
}

export interface PanelData { audio: AudioState | null; sync: SyncSnapshot | null; edge: Edge; notice: string; camBig: boolean; filesTarget: string | null; transfers: Transfer[]; perms: Permission[] | null; prefs: Record<AudioKind, string>; shortcuts: Shortcuts; recording: ActionId | null; mouse: MouseSettings; mice: number | null; tab: SettingsTab; trackpad: PadSettings; padZone: PadZone; pads: number | null; pen: PenStyle; drawing: boolean; batteries: Battery[] | null; toolbar: ToolbarLayout }

export function panelHtml(id: PanelId, d: PanelData): string {
  switch (id) {
    case "mic":
    case "speaker":
      return audioPanel(id, d.audio, d.prefs[id === "mic" ? "input" : "output"]);
    case "bluetooth":
      return bluetoothPanel(d.batteries);
    case "media":
      return mediaPanel();
    case "camera":
      return cameraPanel(d.camBig);
    case "clipboard":
      return clipboardPanel(d.sync);
    case "files":
      return filesPanel(d.sync, d.filesTarget, d.transfers);
    case "settings":
      return settingsPanel(d);
    case "notice":
      return `<p class="notice">${esc(d.notice)}</p>`;
  }
}
