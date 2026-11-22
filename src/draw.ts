// The drawing overlay window: strokes live until the pen is switched off (the window closes).
// Right-click undoes the last stroke.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface DrawStyle { color: string; width: number; highlighter: boolean }
interface Stroke { style: DrawStyle; points: [number, number][] }

const logLine = (msg: string) => void invoke("log_line", { msg: `draw: ${msg}` }).catch(() => {});
const canvas = document.querySelector<HTMLCanvasElement>("#ink")!;
const ctx = canvas.getContext("2d")!;
const strokes: Stroke[] = [];
let style: DrawStyle = { color: "#ff3b30", width: 4, highlighter: false };
let live: Stroke | null = null;

function resize(): void {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(innerWidth * dpr);
  canvas.height = Math.round(innerHeight * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  redraw();
}

function paint(s: Stroke): void {
  const pts = s.points;
  if (!pts.length) return;
  ctx.save();
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.strokeStyle = s.style.color;
  ctx.fillStyle = s.style.color;
  // highlighter: wide and see-through; multiply keeps text under it readable
  ctx.globalAlpha = s.style.highlighter ? 0.35 : 1;
  ctx.globalCompositeOperation = s.style.highlighter ? "multiply" : "source-over";
  ctx.lineWidth = s.style.highlighter ? s.style.width * 4 : s.style.width;
  if (pts.length === 1) {
    ctx.beginPath();
    ctx.arc(pts[0][0], pts[0][1], ctx.lineWidth / 2, 0, Math.PI * 2);
    ctx.fill();
  } else {
    // midpoint quadratic curves: smooth lines without lag
    ctx.beginPath();
    ctx.moveTo(pts[0][0], pts[0][1]);
    for (let i = 1; i < pts.length - 1; i++) {
      const [x, y] = pts[i];
      const [nx, ny] = pts[i + 1];
      ctx.quadraticCurveTo(x, y, (x + nx) / 2, (y + ny) / 2);
    }
    const [lx, ly] = pts[pts.length - 1];
    ctx.lineTo(lx, ly);
    ctx.stroke();
  }
  ctx.restore();
}

function redraw(): void {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  strokes.forEach(paint);
  if (live) paint(live);
}

canvas.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  canvas.setPointerCapture(e.pointerId);
  live = { style: { ...style }, points: [[e.clientX, e.clientY]] };
  redraw();
});
canvas.addEventListener("pointermove", (e) => {
  if (!live) return;
  // coalesced events keep fast strokes from turning into polygons
  const events = e.getCoalescedEvents?.() ?? [e];
  events.forEach((c) => live!.points.push([c.clientX, c.clientY]));
  redraw();
});
const end = () => {
  if (live) strokes.push(live);
  live = null;
};
canvas.addEventListener("pointerup", end);
canvas.addEventListener("pointercancel", end);
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  strokes.pop();
  redraw();
});

addEventListener("resize", resize);
void listen<DrawStyle>("draw-style", ({ payload }) => (style = payload));
void invoke<DrawStyle>("draw_style").then((s) => (style = s), (e) => console.error("[draw] style", e));
resize();

window.addEventListener("error", (e) => logLine(`uncaught ${e.message}`));
logLine(`ready ${innerWidth}x${innerHeight} dpr=${devicePixelRatio}`);
