// Typed wrappers around the Rust commands in src-tauri/src.
import type { padForRust } from "./trackpad-ui";
import { invoke } from "@tauri-apps/api/core";

export type Edge = "left" | "right" | "top" | "bottom";
export type AudioKind = "input" | "output";

export interface Device { id: number; name: string }
export interface Side { devices: Device[]; default: number; muted: boolean; in_use: boolean }
export interface AudioState { input: Side; output: Side; camera_in_use: boolean }

export interface Peer { name: string; ip: string; os: string; online: boolean; dock: boolean; version: string | null }
export interface LastClip { peer: string; incoming: boolean; preview: string; at: number }
export interface Battery { name: string; percent: number; charging: boolean }
export interface PenStyle { color: string; width: number; highlighter: boolean }
export interface MouseSettings { tap: boolean; two_finger_right: boolean; right_side_right: boolean; three_finger_middle: boolean }
export interface SyncSnapshot { enabled: boolean; listening: boolean; version: string; update: Peer | null; me: string | null; peers: Peer[]; last: LastClip | null }

export interface Transfer {
  id: number;
  name: string;
  peer: string;
  incoming: boolean;
  done: number;
  total: number;
  state: "active" | "done" | "failed";
  error: string | null;
  path: string | null;
}

export type PermissionStatus = "granted" | "denied" | "not-asked" | "unknown";
export interface Permission { id: string; name: string; why: string; status: PermissionStatus }

export const api = {
  place: (edge: Edge, width: number, height: number, radius: number, along: number): Promise<void> =>
    invoke("dock_place", { edge, width, height, radius, along }),
  dropped: (): Promise<[Edge, number]> => invoke("dock_dropped"),
  audio: (): Promise<AudioState> => invoke("audio_state"),
  setDefault: (kind: AudioKind, id: number): Promise<void> => invoke("audio_set_default", { kind, id }),
  setMute: (kind: AudioKind, muted: boolean): Promise<boolean> => invoke("audio_set_mute", { kind, muted }),
  media: (key: "play" | "next" | "prev"): Promise<void> => invoke("media_key", { key }),
  batteries: (): Promise<Battery[]> => invoke("batteries"),
  log: (msg: string): Promise<void> => invoke("log_line", { msg }),
  pushSettings: (json: string): Promise<string[]> => invoke("settings_push", { json }),
  meetCamera: (): Promise<string> => invoke("meet_camera"),
  /** Pen overlay on/off; a new style while on applies live. Returns whether it's on. */
  drawSet: (on: boolean, style: PenStyle): Promise<boolean> => invoke("draw_set", { on, style }),
  /** Returns how many Magic Mice are connected. */
  mouseConfigure: (settings: MouseSettings): Promise<number> => invoke("mouse_configure", { settings }),
  mouseStatus: (): Promise<number> => invoke("mouse_status"),
  /** Returns how many trackpads are connected. */
  trackpadConfigure: (settings: ReturnType<typeof padForRust>): Promise<number> => invoke("trackpad_configure", { settings }),
  updateFrom: (ip: string): Promise<void> => invoke("update_from", { ip }),
  sync: (): Promise<SyncSnapshot> => invoke("sync_status"),
  setSync: (enabled: boolean): Promise<SyncSnapshot> => invoke("sync_set_enabled", { enabled }),
  sendFiles: (ip: string, paths: string[]): Promise<void> => invoke("files_send", { ip, paths }),
  reveal: (path: string): Promise<void> => invoke("files_reveal", { path }),
  permissions: (): Promise<Permission[]> => invoke("permissions"),
  openPermission: (id: string): Promise<void> => invoke("permission_open", { id }),
  quit: (): Promise<void> => invoke("quit"),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong";
}
