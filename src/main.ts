import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorText, type AudioKind, type AudioState, type Battery, type Edge, type MouseSettings, type PenStyle, type Permission, type SyncSnapshot, type Transfer } from "./api";
import { startCamera, stopCamera } from "./camera";
import { icons } from "./icons";
import { batteryLevel, camSizeButton, panelHtml, PEN_COLORS, SETTINGS_TABS, transferMeta, type PanelId, type SettingsTab } from "./panels";
import { padForRust, parsePad, setZone, type PadSettings, type PadZone } from "./trackpad-ui";
import { parseToolbar, type ToolbarLayout, type ToolId } from "./toolbar";
import { ACTIONS, accelerator, applyShortcuts, pauseShortcuts, type ActionId, type Shortcuts } from "./shortcuts";

const AUDIO_POLL_MS = 2000;
const NOTICE_MS = 5000;
const REMUTE_CHECK_MS = 1500;
const HANDLE: [number, number] = [22, 76];
const RADIUS = { handle: 11, bar: 22, panel: 26 };
const EDGES: Edge[] = ["left", "right", "top", "bottom"];
const MAX_TRANSFERS = 6;
const BATTERY_POLL_MS = 60_000;
// Stored settings that travel to other devices (not per-run state like the open tab or camera size).
const SYNCED_KEYS = ["edge", "along", "toolbar", "shortcuts", "mouse", "trackpad", "pen", "pref.input", "pref.output"];

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector<T>(sel)!;
const app = $("#app");
const bar = $("#bar");
const panel = $("#panel");
const handle = $("#handle");
const tool = (name: string) => bar.querySelector<HTMLButtonElement>(`[data-tool="${name}"]`)!;

function load(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}
function save(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage blocked: setting lasts for this run only */
  }
}

const savedEdge = load("edge", "left") as Edge;
const state = {
  edge: EDGES.includes(savedEdge) ? savedEdge : "left",
  along: parseAlong(load("along", "0.5")), // where along the edge: 0 = top/left end, 1 = bottom/right end
  expanded: load("expanded", "1") === "1",
  panel: null as PanelId | null,
  audio: null as AudioState | null,
  sync: null as SyncSnapshot | null,
  notice: "",
  camBig: load("camBig", "0") === "1",
  filesTarget: load("filesTarget", "") || null,
  transfers: [] as Transfer[],
  perms: null as Permission[] | null,
  // Starred device name per kind ("" = none). Names, not ids: CoreAudio ids change on every reconnect.
  prefs: { input: load("pref.input", ""), output: load("pref.output", "") } as Record<AudioKind, string>,
  shortcuts: parseShortcuts(load("shortcuts", "{}")),
  toolbar: parseToolbar(load("toolbar", "")),
  recording: null as ActionId | null,
  mouse: parseMouse(load("mouse", "")),
  mice: null as number | null,
  tab: (SETTINGS_TABS.some(([id]) => id === load("settingsTab", "")) ? load("settingsTab", "") : "general") as SettingsTab,
  trackpad: parsePad(load("trackpad", "{}")),
  padZone: "right" as PadZone,
  pads: null as number | null,
  pen: parsePen(load("pen", "")),
  drawing: false,
  batteries: null as Battery[] | null,
};

function parseAlong(raw: string): number {
  const v = Number(raw);
  return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0.5;
}

function parsePen(raw: string): PenStyle {
  const base: PenStyle = { color: PEN_COLORS[0], width: 4, highlighter: false };
  try {
    const v = JSON.parse(raw) as Partial<PenStyle>;
    return {
      color: typeof v.color === "string" && /^#[0-9a-f]{6}$/i.test(v.color) ? v.color : base.color,
      width: typeof v.width === "number" && v.width >= 1 && v.width <= 24 ? Math.round(v.width) : base.width,
      highlighter: typeof v.highlighter === "boolean" ? v.highlighter : base.highlighter,
    };
  } catch {
    return base;
  }
}

// Defaults match the old Magic Mouse Click app: tap = click, two-finger tap = right-click.
function parseMouse(raw: string): MouseSettings {
  const base: MouseSettings = { tap: true, two_finger_right: true, right_side_right: false, three_finger_middle: false };
  try {
    const v: unknown = JSON.parse(raw);
    if (!v || typeof v !== "object") return base;
    const picked = Object.entries(v).filter(([k, on]) => k in base && typeof on === "boolean");
    return { ...base, ...Object.fromEntries(picked) };
  } catch {
    return base;
  }
}

function parseShortcuts(raw: string): Shortcuts {
  try {
    const v: unknown = JSON.parse(raw);
    if (!v || typeof v !== "object") return {};
    return Object.fromEntries(
      Object.entries(v).filter(([id, a]) => ACTIONS.some((x) => x.id === id) && typeof a === "string"),
    ) as Shortcuts;
  } catch {
    return {};
  }
}

// ---------- placement ----------
let lastPlacement = "";
let placeQueued = false;

function measure(): [number, number, number] {
  const vertical = state.edge === "left" || state.edge === "right";
  if (!state.expanded) return vertical ? [HANDLE[0], HANDLE[1], RADIUS.handle] : [HANDLE[1], HANDLE[0], RADIUS.handle];
  const [bw, bh] = [bar.offsetWidth, bar.offsetHeight];
  if (!state.panel) return [bw, bh, RADIUS.bar];
  const [pw, ph] = [panel.offsetWidth, panel.offsetHeight];
  return vertical ? [bw + pw, Math.max(bh, ph), RADIUS.panel] : [Math.max(bw, pw), bh + ph, RADIUS.panel];
}

function place(): void {
  if (placeQueued) return;
  placeQueued = true;
  // microtask, not rAF: rAF never fires while the window is still hidden at boot
  queueMicrotask(() => {
    placeQueued = false;
    const [w, h, r] = measure();
    const key = `${state.edge}:${state.along}:${w}x${h}:${r}`;
    if (key === lastPlacement || w === 0) return;
    lastPlacement = key;
    app.style.setProperty("--r", `${r}px`);
    api.place(state.edge, w, h, r, state.along).catch((e) => console.error("[dock] place failed", e));
  });
}
new ResizeObserver(place).observe(bar);
new ResizeObserver(place).observe(panel);

// ---------- rendering ----------
function renderBar(): void {
  const micMuted = state.audio?.input.muted ?? false;
  const outMuted = state.audio?.output.muted ?? false;
  app.className = [`edge-${state.edge}`, state.expanded ? "" : "collapsed", micMuted ? "mic-muted" : ""].join(" ");
  handle.setAttribute("aria-label", micMuted ? "Expand dock (mic muted)" : "Expand dock");

  const micLive = !!state.audio?.input.in_use && !micMuted;
  app.classList.toggle("mic-live", micLive);
  paint("mic", micMuted ? "micOff" : "mic", { "live-dot": micLive });
  tool("mic").classList.toggle("muted", micMuted);
  paint("speaker", outMuted ? "speakerOff" : "speaker", {});
  tool("speaker").classList.toggle("muted", outMuted);
  paint("media", "playPause", { eq: !!state.audio?.output.in_use && !outMuted });
  paint("camera", "camera", { "live-dot": !!state.audio?.camera_in_use });
  paint("pen", "pen", {});
  // the dot takes the colour of the weakest accessory (the list arrives lowest first)
  const weakest = batteryLevel(state.batteries?.[0]?.percent ?? 100);
  paint("bluetooth", "bluetooth", { "batt-dot": !!weakest });
  tool("bluetooth").querySelector(".batt-dot")?.setAttribute("data-level", weakest);
  tool("bluetooth").title = state.batteries?.[0] && weakest ? `${state.batteries[0].name}: ${state.batteries[0].percent}%` : "Bluetooth batteries";
  tool("pen").classList.toggle("on", state.drawing);
  paint("files", "files", { "live-dot": state.transfers.some((t) => t.state === "active") });
  paint("settings", "sliders", { badge: !!state.sync?.update });
  paint("grip", "grip", {});

  const sync = state.sync;
  const live = !!sync?.enabled && !!sync.peers.some((p) => p.dock);
  paint("clipboard", "clipboard", { badge: !!sync });
  tool("clipboard").querySelector(".badge")?.classList.toggle("off", !live);
  bar.querySelectorAll(".tool").forEach((b) => b.classList.toggle("open", (b as HTMLElement).dataset.tool === state.panel));
  place();
}

const MARKS: Record<string, string> = {
  "live-dot": `<span class="live-dot" aria-hidden="true"></span>`,
  eq: `<span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>`,
  badge: `<span class="badge" aria-hidden="true"></span>`,
  "batt-dot": `<span class="batt-dot" aria-hidden="true"></span>`,
};

/** Update a tool's icon and indicator marks in place, so running animations aren't restarted. */
function paint(name: string, icon: keyof typeof icons, marks: Record<string, boolean>): void {
  const b = tool(name);
  let ico = b.querySelector<HTMLElement>(":scope > .ico");
  if (!ico) {
    ico = document.createElement("span");
    ico.className = "ico";
    b.prepend(ico);
  }
  if (ico.dataset.icon !== icon) {
    ico.innerHTML = icons[icon];
    ico.dataset.icon = icon;
  }
  for (const [cls, on] of Object.entries(marks)) {
    const el = b.querySelector(`:scope > .${cls}`);
    if (on && !el) b.insertAdjacentHTML("beforeend", MARKS[cls]);
    if (!on && el) el.remove();
  }
}

function pulse(name: string): void {
  const b = tool(name);
  b.classList.remove("pulse");
  void b.offsetWidth; // restart the animation
  b.classList.add("pulse");
}

function renderPanel(): void {
  const id = state.panel;
  panel.hidden = !id || !state.expanded;
  if (!id) {
    stopCamera();
    panel.innerHTML = "";
    return;
  }
  panel.className = `panel panel-${id}${id === "camera" || id === "settings" || id === "bluetooth" ? " wide" : ""}${id === "camera" && state.camBig ? " big" : ""}`;
  panel.innerHTML = panelHtml(id, state);
  if (id === "camera") void startCamera(panel, place);
  else stopCamera();
}

function refreshPanelIf(...ids: PanelId[]): void {
  if (state.panel && ids.includes(state.panel)) renderPanel();
}

function openPanel(id: PanelId | null): void {
  state.panel = state.panel === id ? null : id;
  if (state.panel === "settings") {
    void run(refreshPerms);
    void run(async () => {
      state.mice = await api.mouseStatus(); // Bluetooth mice come and go
      refreshPanelIf("settings");
    });
  }
  renderPanel();
  renderBar();
}

let noticeTimer = 0;
function notice(msg: string): void {
  state.notice = msg;
  state.panel = null;
  openPanel("notice");
  clearTimeout(noticeTimer);
  noticeTimer = window.setTimeout(() => state.panel === "notice" && openPanel(null), NOTICE_MS);
}

function setExpanded(expanded: boolean): void {
  state.expanded = expanded;
  save("expanded", expanded ? "1" : "0");
  if (!expanded) state.panel = null;
  renderPanel();
  renderBar();
}

// ---------- actions ----------
const prefSeen: Record<AudioKind, boolean> = { input: false, output: false };

// Switch to the starred device when it shows up (launch, plug-in). Picking another device while it stays connected sticks.
async function applyPreferred(a: AudioState): Promise<boolean> {
  let switched = false;
  for (const kind of ["input", "output"] as const) {
    const side = kind === "input" ? a.input : a.output;
    const dev = side.devices.find((d) => d.name === state.prefs[kind]);
    const appeared = !!dev && !prefSeen[kind];
    prefSeen[kind] = !!dev;
    if (dev && appeared && dev.id !== side.default) {
      await api.setDefault(kind, dev.id);
      switched = true;
    }
  }
  return switched;
}

function toggleStar(kind: AudioKind, name: string): void {
  const next = state.prefs[kind] === name ? "" : name;
  state.prefs = { ...state.prefs, [kind]: next };
  save(`pref.${kind}`, next);
  prefSeen[kind] = false; // starring a connected device switches to it now
  refreshPanelIf("mic", "speaker");
  void refreshAudio();
}

// ---------- magic mouse ----------
async function setMouse(next: MouseSettings): Promise<void> {
  state.mouse = next;
  save("mouse", JSON.stringify(next));
  state.mice = await api.mouseConfigure(next);
  refreshPanelIf("settings");
}

// ---------- pen ----------
async function setDrawing(on: boolean): Promise<void> {
  state.drawing = await api.drawSet(on, state.pen);
  renderBar();
  refreshPanelIf("settings");
}

async function setPen(next: PenStyle, rerender = true): Promise<void> {
  state.pen = next;
  save("pen", JSON.stringify(next));
  if (state.drawing) await api.drawSet(true, next); // live: the next stroke uses it
  if (rerender) refreshPanelIf("settings");
}

function openPenSettings(): void {
  state.tab = "draw";
  if (state.panel !== "settings") openPanel("settings");
  else refreshPanelIf("settings");
}

// ---------- trackpad ----------
async function setTrackpad(next: PadSettings): Promise<void> {
  state.trackpad = next;
  save("trackpad", JSON.stringify(next));
  state.pads = await api.trackpadConfigure(padForRust(next));
  refreshPanelIf("settings");
}

// ---------- shortcuts ----------
function runShortcut(id: ActionId): void {
  if (id === "mic") void toggleMute("input");
  else if (id === "speaker") void toggleMute("output");
  else if (id === "play" || id === "next" || id === "prev") void run(() => api.media(id));
  else if (id === "meet") void run(async () => notice(await api.meetCamera()));
  else if (id === "draw") void run(() => setDrawing(!state.drawing));
  else if (id === "mouse") {
    const on = !state.mouse.tap;
    void run(() => setMouse({ ...state.mouse, tap: on }));
    notice(`Magic Mouse tap to click ${on ? "on" : "off"}`);
  } else if (id === "mirror") {
    if (!state.expanded) setExpanded(true);
    openPanel("camera");
  }
}

async function registerShortcuts(): Promise<void> {
  const failed = await applyShortcuts(state.shortcuts, runShortcut);
  const names = ACTIONS.filter((a) => failed.includes(a.id)).map((a) => a.label);
  if (names.length) notice(`Shortcut already used by another app: ${names.join(", ")}`);
}

function startRecording(id: ActionId): void {
  state.recording = state.recording === id ? null : id;
  void getCurrentWindow().setFocus(); // keystrokes only reach Ledge while its window is key
  void (state.recording ? pauseShortcuts() : registerShortcuts());
  refreshPanelIf("settings");
}

function onRecordKey(e: KeyboardEvent): void {
  const id = state.recording;
  if (!id) return;
  e.preventDefault();
  const plain = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
  let next: Shortcuts | null = null;
  if (e.key === "Escape" && plain) next = state.shortcuts;
  else if ((e.key === "Backspace" || e.key === "Delete") && plain) next = { ...state.shortcuts, [id]: undefined };
  else {
    const accel = accelerator(e);
    if (!accel) return; // still holding modifiers
    // one combo, one action: taking it from another action moves it here
    const others = Object.fromEntries(Object.entries(state.shortcuts).filter(([, a]) => a !== accel));
    next = { ...others, [id]: accel };
  }
  state.shortcuts = next;
  state.recording = null;
  save("shortcuts", JSON.stringify(next));
  refreshPanelIf("settings");
  void registerShortcuts();
}

async function refreshAudio(): Promise<void> {
  try {
    let next = await api.audio();
    if (await applyPreferred(next)) next = await api.audio();
    if (JSON.stringify(next) === JSON.stringify(state.audio)) return;
    state.audio = next;
    renderBar();
    refreshPanelIf("mic", "speaker");
  } catch (e) {
    console.error("[dock] audio state failed", e);
  }
}

async function toggleMute(kind: AudioKind): Promise<void> {
  const side = kind === "input" ? state.audio?.input : state.audio?.output;
  const unmuting = side?.muted ?? false;
  try {
    await api.setMute(kind, !unmuting);
  } catch (e) {
    notice(errorText(e));
  }
  await refreshAudio();
  if (!unmuting) return;
  // Some other app (mic-mute utility, dictation tool, macro pad) may force it back — say so instead of flipping silently.
  await new Promise((r) => setTimeout(r, REMUTE_CHECK_MS));
  await refreshAudio();
  const now = kind === "input" ? state.audio?.input : state.audio?.output;
  if (now?.muted) notice(`Another app muted the ${kind === "input" ? "mic" : "output"} again right after Ledge unmuted it.`);
}

async function run(action: () => Promise<unknown>): Promise<void> {
  try {
    await action();
  } catch (e) {
    notice(errorText(e));
    void api.log(`error: ${errorText(e)}`).catch(() => {});
  }
}

// resize in place: re-rendering the panel would restart the camera stream
function toggleCamSize(btn: HTMLElement): void {
  state.camBig = !state.camBig;
  save("camBig", state.camBig ? "1" : "0");
  panel.classList.toggle("big", state.camBig);
  btn.outerHTML = camSizeButton(state.camBig);
}

// ---------- permissions ----------
async function refreshPerms(): Promise<void> {
  const next = await api.permissions();
  if (JSON.stringify(next) === JSON.stringify(state.perms)) return;
  state.perms = next;
  refreshPanelIf("settings");
}

// Never-asked camera: Rust shows macOS's prompt and emits "permissions" once answered. Otherwise opens Settings.
async function grant(id: string): Promise<void> {
  await api.openPermission(id);
  await refreshPerms();
}

// ---------- files ----------
function sendTo(paths: string[]): void {
  const target = state.filesTarget;
  if (!paths.length) return;
  if (!target) return notice("Pick a device in Send files first");
  void run(() => api.sendFiles(target, paths));
}

async function browse(): Promise<void> {
  const picked = await openDialog({ multiple: true, directory: false, title: "Send files" });
  if (picked) sendTo(Array.isArray(picked) ? picked : [picked]);
}

function showFiles(): void {
  if (!state.expanded) setExpanded(true);
  if (state.panel !== "files") openPanel("files");
}

function onTransfer(t: Transfer): void {
  const known = state.transfers.some((x) => x.id === t.id && x.incoming === t.incoming);
  state.transfers = known
    ? state.transfers.map((x) => (x.id === t.id && x.incoming === t.incoming ? t : x))
    : [t, ...state.transfers].slice(0, MAX_TRANSFERS);
  // progress ticks update the row in place; anything else re-renders the panel
  const row = panel.querySelector<HTMLElement>(`[data-transfer="${t.id}"]`);
  if (state.panel === "files" && row && known && t.state === "active") {
    row.querySelector<HTMLElement>(".fill")!.style.transform = `scaleX(${t.total ? t.done / t.total : 1})`;
    row.querySelector<HTMLElement>(".meta")!.textContent = transferMeta(t);
  } else {
    refreshPanelIf("files");
  }
  if (t.state === "done" && t.incoming) pulse("files");
  if (t.state === "failed" && state.panel !== "files") notice(`${t.name}: ${t.error ?? "failed"}`);
  renderBar();
}

function setEdge(edge: Edge, along = 0.5): void {
  state.edge = edge;
  state.along = along;
  save("edge", edge);
  save("along", String(along));
  renderBar();
  renderPanel();
}

// ---------- events ----------
document.addEventListener("contextmenu", (e) => e.preventDefault());
document.addEventListener("keydown", (e) => {
  if (state.recording) return onRecordKey(e);
  if (e.key === "Tab") document.body.classList.add("kbd");
});
// mouse clicks shouldn't leave focus rings; keyboard focus still shows them
document.addEventListener("mousedown", (e) => (e.target as HTMLElement).closest("button") && e.preventDefault());
handle.addEventListener("click", () => setExpanded(true));
tool("grip").addEventListener("mousedown", startDrag);

// ---------- toolbar layout ----------
/** Puts the bar's tools in the layout's order (before the settings separator) and hides switched-off ones. */
function applyToolbar(t: ToolbarLayout): void {
  const end = bar.querySelector("[data-tools-end]");
  for (const id of t.order) {
    const b = tool(id);
    bar.insertBefore(b, end);
    b.hidden = t.hidden.includes(id);
  }
}

function setToolbar(t: ToolbarLayout): void {
  state.toolbar = t;
  save("toolbar", JSON.stringify(t));
  applyToolbar(t);
  refreshPanelIf("settings");
}

function toggleTool(id: ToolId): void {
  const hidden = state.toolbar.hidden.includes(id) ? state.toolbar.hidden.filter((h) => h !== id) : [...state.toolbar.hidden, id];
  setToolbar({ ...state.toolbar, hidden });
}

function moveTool(id: ToolId, by: number): void {
  const order = state.toolbar.order.filter((o) => o !== id);
  const at = Math.max(0, Math.min(order.length, state.toolbar.order.indexOf(id) + by));
  setToolbar({ ...state.toolbar, order: [...order.slice(0, at), id, ...order.slice(at)] });
  panel.querySelector<HTMLElement>(`[data-toolbar="${id}"]`)?.focus();
}

/** Pointer events, not HTML drag-and-drop: the window's file-drop handler swallows HTML5 drags on Windows.
 * Listens on the window: moving the row in the DOM drops any pointer capture held by its grip. */
function startReorder(e: PointerEvent, grip: HTMLElement): void {
  const row = grip.closest<HTMLElement>("[data-tool-row]");
  const list = row?.parentElement;
  if (!row || !list) return;
  e.preventDefault();
  row.classList.add("dragging");
  const domOrder = () => Array.from(list.querySelectorAll<HTMLElement>("[data-tool-row]")).map((r) => r.dataset.toolRow as ToolId);
  const move = (ev: PointerEvent) => {
    const others = Array.from(list.querySelectorAll<HTMLElement>("[data-tool-row]:not(.dragging)"));
    const before = others.find((r) => { const b = r.getBoundingClientRect(); return ev.clientY < b.top + b.height / 2; });
    if (before === row.nextElementSibling) return;
    list.insertBefore(row, before ?? null);
    applyToolbar({ ...state.toolbar, order: domOrder() }); // the bar follows live
  };
  const done = () => {
    removeEventListener("pointermove", move);
    removeEventListener("pointerup", done);
    removeEventListener("pointercancel", done);
    removeEventListener("blur", done);
    setToolbar({ ...state.toolbar, order: domOrder() });
  };
  addEventListener("pointermove", move);
  addEventListener("pointerup", done);
  addEventListener("pointercancel", done);
  addEventListener("blur", done); // released outside the window
}

panel.addEventListener("pointerdown", (e) => {
  const grip = (e.target as HTMLElement).closest<HTMLElement>("[data-drag]");
  if (grip && e.button === 0) startReorder(e, grip);
});
panel.addEventListener("keydown", (e) => {
  const id = (e.target as HTMLElement).closest<HTMLElement>("[data-toolbar]")?.dataset.toolbar as ToolId | undefined;
  if (!id || !e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
  e.preventDefault();
  moveTool(id, e.key === "ArrowUp" ? -1 : 1);
});
applyToolbar(state.toolbar);

bar.addEventListener("click", (e) => {
  const name = (e.target as HTMLElement).closest<HTMLElement>("[data-tool]")?.dataset.tool;
  if (name === "collapse") setExpanded(false);
  else if (name === "mic") void toggleMute("input");
  else if (name === "speaker") void toggleMute("output");
  else if (name === "media") void run(() => api.media("play"));
  else if (name === "camera") void run(async () => notice(await api.meetCamera()));
  else if (name === "pen") void run(() => setDrawing(!state.drawing));
  else if (name === "bluetooth") {
    openPanel(name);
    void run(refreshBatteries);
const syncedFrom = load("syncedFrom", "");
if (syncedFrom) {
  save("syncedFrom", "");
  notice(`Settings synced from ${syncedFrom}`);
}
  } else if (name === "clipboard" || name === "files" || name === "settings") openPanel(name);
});

bar.addEventListener("contextmenu", (e) => {
  const name = (e.target as HTMLElement).closest<HTMLElement>("[data-tool]")?.dataset.tool;
  if (name === "mic" || name === "speaker" || name === "media" || name === "camera") openPanel(name);
  else if (name === "pen") openPenSettings();
});

// Range and color inputs report while dragging: update in place, re-render only when done.
panel.addEventListener("input", (e) => {
  const el = e.target as HTMLInputElement;
  if (el.matches("[data-pen-width]")) {
    const out = panel.querySelector("[data-pen-width-out]");
    if (out) out.textContent = `${el.value} px`;
    panel.querySelector<HTMLElement>(".pen-preview")?.style.setProperty("--w", `${el.value}px`);
  }
});
panel.addEventListener("change", (e) => {
  const el = e.target as HTMLInputElement;
  if (el.matches("[data-pen-width]")) void run(() => setPen({ ...state.pen, width: Number(el.value) }));
  else if (el.matches("[data-pen-custom]") && /^#[0-9a-f]{6}$/i.test(el.value)) void run(() => setPen({ ...state.pen, color: el.value }));
});

panel.addEventListener("click", (e) => {
  const t = (e.target as HTMLElement).closest<HTMLElement>("[data-pen-color],[data-pen-hl],[data-toolbar],[data-tab],[data-zone],[data-pad-value],[data-mouse],[data-shortcut],[data-star],[data-device],[data-action],[data-edge],[data-media],[data-peer],[data-reveal],[data-perm]");
  if (!t) return;
  const kind: AudioKind = state.panel === "mic" ? "input" : "output";
  const { device, action, edge, media, peer, reveal, perm, star, shortcut, mouse, tab, zone, padValue, penColor, toolbar } = t.dataset;
  if (toolbar) toggleTool(toolbar as ToolId);
  else if (penColor) void run(() => setPen({ ...state.pen, color: penColor }));
  else if (t.hasAttribute("data-pen-hl")) void run(() => setPen({ ...state.pen, highlighter: !state.pen.highlighter }));
  else if (action === "pen") void run(() => setDrawing(!state.drawing));
  else if (tab) {
    state.tab = tab as SettingsTab;
    state.recording = null; // leaving Shortcuts mid-recording cancels it
    save("settingsTab", tab);
    void registerShortcuts();
    refreshPanelIf("settings");
  } else if (zone) {
    state.padZone = zone as PadZone;
    refreshPanelIf("settings");
  } else if (padValue !== undefined) void run(() => setTrackpad(setZone(state.trackpad, state.padZone, padValue)));
  else if (mouse && mouse in state.mouse) {
    const key = mouse as keyof MouseSettings;
    void run(() => setMouse({ ...state.mouse, [key]: !state.mouse[key] }));
  } else if (shortcut) startRecording(shortcut as ActionId);
  else if (star !== undefined) toggleStar(kind, star);
  else if (perm) void run(() => grant(perm));
  else if (peer) {
    state.filesTarget = peer;
    save("filesTarget", peer);
    refreshPanelIf("files");
  } else if (reveal) void run(() => api.reveal(reveal));
  else if (action === "browse") void run(browse);
  else if (device) void run(() => api.setDefault(kind, Number(device))).then(refreshAudio);
  else if (action === "mute") void toggleMute(kind);
  else if (action === "sync") void run(async () => {
    state.sync = await api.setSync(!state.sync?.enabled);
    save("sync", state.sync.enabled ? "1" : "0");
    renderBar();
    refreshPanelIf("clipboard");
  });
  else if (action === "cam-size") toggleCamSize(t);
  else if (action === "quit") void api.quit();
  else if (action === "sync-settings") void run(pushSettings);
  else if (action === "update") installUpdate();
  else if (edge) setEdge(edge as Edge);
  else if (media === "play" || media === "next" || media === "prev") void run(() => api.media(media));
});

async function pushSettings(): Promise<void> {
  const all = Object.fromEntries(SYNCED_KEYS.map((k) => [k, load(k, "")]).filter(([, v]) => v !== ""));
  const took = await api.pushSettings(JSON.stringify(all));
  notice(`Settings sent to ${took.join(", ")}`);
}

/** Settings from another device: store the known keys, then restart the page so everything re-applies. */
async function takeSettings(from: string, json: string): Promise<void> {
  const v = JSON.parse(json) as Record<string, unknown>;
  for (const k of SYNCED_KEYS) {
    const val = v[k];
    if (typeof val === "string") save(k, val);
  }
  if (state.drawing) await setDrawing(false); // the overlay would outlive the reload
  save("syncedFrom", from);
  location.reload();
}

async function refreshBatteries(): Promise<void> {
  state.batteries = await api.batteries();
  renderBar();
  refreshPanelIf("bluetooth");
}

// ---------- moving the dock ----------
// The grip starts a native window drag; once moves settle, the dock snaps to the nearest edge of
// whichever screen it was dropped on, keeping the spot along that edge.
const DROP_SETTLE_MS = 250;
const DRAG_GIVE_UP_MS = 4000; // a click without a drag must not leave us listening
let dragging = false;
let dropTimer = 0;
let dragGiveUp = 0;

function startDrag(e: MouseEvent): void {
  if (e.button !== 0) return;
  dragging = true;
  clearTimeout(dragGiveUp);
  dragGiveUp = window.setTimeout(() => (dragging = false), DRAG_GIVE_UP_MS);
  void getCurrentWindow().startDragging().catch((err) => notice(errorText(err)));
}

void getCurrentWindow().onMoved(() => {
  if (!dragging) return;
  clearTimeout(dragGiveUp);
  clearTimeout(dropTimer);
  dropTimer = window.setTimeout(() => {
    dragging = false;
    void run(async () => {
      const [edge, along] = await api.dropped();
      lastPlacement = ""; // same edge and size still has to move
      setEdge(edge, along);
    });
  }, DROP_SETTLE_MS);
});

// ---------- self-update ----------
let updateTried = ""; // auto-install each version once per run, so a bad download can't loop

function installUpdate(): void {
  const u = state.sync?.update;
  if (!u) return;
  updateTried = u.version ?? "";
  notice(`Updating to Ledge ${u.version} from ${u.name}… it relaunches by itself.`);
  void run(() => api.updateFrom(u.ip));
}

function autoUpdate(): void {
  const u = state.sync?.update;
  // the mirror would vanish mid-use; the next sync event retries once it's closed
  if (u && u.version !== updateTried && state.panel !== "camera") installUpdate();
}

void listen<SyncSnapshot>("sync", ({ payload }) => {
  const prevAt = state.sync?.last?.at ?? 0;
  state.sync = payload;
  autoUpdate();
  renderBar();
  refreshPanelIf("clipboard", "settings");
  if (payload.last?.incoming && payload.last.at > prevAt) pulse("clipboard");
});

void listen<Transfer>("transfer", ({ payload }) => onTransfer(payload));
void listen<{ from: string; json: string }>("settings-sync", ({ payload }) => void run(() => takeSettings(payload.from, payload.json)));
void listen("permissions", () => void run(refreshPerms));
void listen<number>("trackpad-corner", ({ payload }) => {
  const id = state.trackpad.corners[payload];
  if (id) runShortcut(id);
});

// Native drag & drop (Tauri delivers real file paths): dragging over the dock opens Send files.
void getCurrentWebview().onDragDropEvent(({ payload }) => {
  const zone = () => panel.querySelector(".drop");
  if (payload.type === "enter" || payload.type === "over") {
    showFiles();
    zone()?.classList.add("hot");
  } else if (payload.type === "leave") {
    zone()?.classList.remove("hot");
  } else if (payload.type === "drop") {
    zone()?.classList.remove("hot");
    sendTo(payload.paths);
  }
});

// ---------- boot ----------
// the native glass layer is macOS-only; elsewhere CSS paints the surface
document.documentElement.classList.toggle("no-glass", !navigator.userAgent.includes("Mac"));
window.addEventListener("error", (e) => void api.log(`uncaught: ${e.message} @ ${e.filename}:${e.lineno}`).catch(() => {}));
window.addEventListener("unhandledrejection", (e) => void api.log(`rejection: ${errorText(e.reason)}`).catch(() => {}));
tool("collapse").innerHTML = icons.chevron;
handle.insertAdjacentHTML("afterbegin", icons.chevron);
renderBar();
(document.activeElement as HTMLElement | null)?.blur();
void refreshAudio();
void registerShortcuts();
void run(() => setMouse(state.mouse));
void run(() => setTrackpad(state.trackpad));
void run(refreshBatteries);
setInterval(() => void refreshBatteries().catch((e) => console.error("[dock] batteries failed", e)), BATTERY_POLL_MS);
setInterval(() => {
  void refreshAudio();
  // picks up toggles made in System Settings while the panel is open
  if (state.panel === "settings") void refreshPerms().catch((e) => console.error("[dock] permissions failed", e));
}, AUDIO_POLL_MS); // shortcut: polling, swap for CoreAudio property listeners if it ever matters
void run(async () => {
  state.sync = load("sync", "1") === "1" ? await api.sync() : await api.setSync(false);
  renderBar();
});
