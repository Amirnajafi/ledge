// Webcam preview with a device picker. The stream only lives while the camera panel is open.
import { errorText } from "./api";

const PREF_KEY = "camera";
let stream: MediaStream | null = null;

export function stopCamera(): void {
  stream?.getTracks().forEach((t) => t.stop());
  stream = null;
}

function preferred(): string | null {
  try {
    return localStorage.getItem(PREF_KEY);
  } catch {
    return null;
  }
}

function remember(id: string): void {
  try {
    localStorage.setItem(PREF_KEY, id);
  } catch {
    /* storage blocked: preference just isn't remembered */
  }
}

async function open(deviceId: string | null): Promise<MediaStream> {
  const video: MediaTrackConstraints = { width: { ideal: 1280 }, height: { ideal: 720 } };
  if (deviceId) {
    try {
      return await navigator.mediaDevices.getUserMedia({ video: { ...video, deviceId: { exact: deviceId } } });
    } catch {
      /* saved camera unplugged — fall through to default */
    }
  }
  return navigator.mediaDevices.getUserMedia({ video });
}

// WebKit says "Invalid constraint" when macOS hides every camera from the app, i.e. no camera permission.
function cameraError(e: unknown): string {
  const name = e instanceof DOMException || e instanceof Error ? e.name : "";
  if (name === "OverconstrainedError" || name === "NotAllowedError")
    return "Ledge has no camera access. Settings → Permissions → Camera.";
  return `Camera unavailable: ${errorText(e)}. Check System Settings → Privacy & Security → Camera.`;
}

/** Starts the preview inside `root` (camera panel markup) and lists cameras to switch between. */
export async function startCamera(root: HTMLElement, onLayout: () => void, deviceId = preferred()): Promise<void> {
  const videoEl = root.querySelector("video");
  const hint = root.querySelector<HTMLElement>("[data-cam-hint]");
  const list = root.querySelector<HTMLElement>("[data-cams]");
  if (!videoEl || !hint || !list) return;
  stopCamera();
  try {
    stream = await open(deviceId);
    if (!root.isConnected) return stopCamera(); // panel closed while we waited
    videoEl.srcObject = stream;
    hint.hidden = true;
  } catch (e) {
    hint.textContent = cameraError(e);
    return;
  }
  const activeId = stream.getVideoTracks()[0]?.getSettings().deviceId ?? "";
  const cams = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "videoinput");
  list.replaceChildren(
    ...cams.map((cam, i) => {
      const row = document.createElement("button");
      row.className = `row${cam.deviceId === activeId ? " current" : ""}`;
      row.innerHTML = `<span class="check">${cam.deviceId === activeId ? "✓" : ""}</span><span class="name"></span>`;
      row.querySelector(".name")!.textContent = cam.label || `Camera ${i + 1}`;
      row.addEventListener("click", () => {
        remember(cam.deviceId);
        void startCamera(root, onLayout, cam.deviceId);
      });
      return row;
    }),
  );
  onLayout();
}
