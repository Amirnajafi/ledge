// Inline SVG icons (24×24, stroke = currentColor) drawn to sit next to SF Symbols.
const svg = (body: string) =>
  `<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;

const micBody = `<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0"/><path d="M12 17.5V21"/>`;
const speakerBody = `<path d="M4 9.5h3l5-4v13l-5-4H4z" fill="currentColor" fill-opacity=".18"/>`;

export const icons = {
  chevron: svg(`<path d="M9.5 6l6 6-6 6"/>`),
  mic: svg(micBody),
  micOff: svg(`${micBody}<path d="M4 4l16 16" stroke-width="2.2"/>`),
  speaker: svg(`${speakerBody}<path d="M15.5 9.5a3.5 3.5 0 0 1 0 5"/><path d="M18 7a7 7 0 0 1 0 10"/>`),
  speakerOff: svg(`${speakerBody}<path d="M16 9.5l5 5M21 9.5l-5 5"/>`),
  playPause: svg(`<path d="M3.5 6.5l7 5.5-7 5.5z" fill="currentColor"/><path d="M15 6.5v11M19.5 6.5v11" stroke-width="2.4"/>`),
  next: svg(`<path d="M5 6.5l8 5.5-8 5.5z" fill="currentColor"/><path d="M17.5 6.5v11" stroke-width="2.2"/>`),
  prev: svg(`<path d="M19 6.5l-8 5.5 8 5.5z" fill="currentColor"/><path d="M6.5 6.5v11" stroke-width="2.2"/>`),
  camera: svg(`<rect x="2.5" y="7" width="13.5" height="10" rx="2.5"/><path d="M16 10.5l5-3v9l-5-3z"/>`),
  clipboard: svg(
    `<rect x="5.5" y="4.5" width="13" height="16.5" rx="2.5"/><rect x="9" y="2.5" width="6" height="3.5" rx="1.2" fill="currentColor" fill-opacity=".18"/><path d="M9 12.5h6M9 16h4"/>`,
  ),
  grip: svg(`<g fill="currentColor" stroke="none"><circle cx="9" cy="6" r="1.6"/><circle cx="15" cy="6" r="1.6"/><circle cx="9" cy="12" r="1.6"/><circle cx="15" cy="12" r="1.6"/><circle cx="9" cy="18" r="1.6"/><circle cx="15" cy="18" r="1.6"/></g>`),
  sliders: svg(
    `<path d="M4 7.5h9M17 7.5h3M4 16.5h3M11 16.5h9"/><circle cx="15" cy="7.5" r="2"/><circle cx="9" cy="16.5" r="2"/>`,
  ),
  files: svg(`<path d="M12 15V4M7.5 8.5L12 4l4.5 4.5"/><path d="M4 13.5v5A1.5 1.5 0 0 0 5.5 20h13a1.5 1.5 0 0 0 1.5-1.5v-5"/>`),
  enlarge: svg(`<path d="M14 4h6v6M10 20H4v-6M20 4l-6.5 6.5M4 20l6.5-6.5"/>`),
  shrink: svg(`<path d="M20 10h-6V4M4 14h6v6M14 10l6.5-6.5M10 14l-6.5 6.5"/>`),
  power: svg(`<path d="M12 3.5v8"/><path d="M7 6.5a7 7 0 1 0 10 0"/>`),
  bluetooth: svg(`<path d="M7 7.5l10 9-5 4.5V3l5 4.5-10 9"/>`),
  pen: svg(`<path d="M15.5 4.5l4 4L9 19l-5 1 1-5z" fill="currentColor" fill-opacity=".14"/><path d="M13.5 6.5l4 4"/>`),
  sun: svg(`<circle cx="12" cy="12" r="3.6" fill="currentColor" fill-opacity=".18"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4"/>`),
  keyboard: svg(`<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M7.5 14h9"/>`),
  mouse: svg(`<rect x="6.5" y="3" width="11" height="18" rx="5.5"/><path d="M12 6.5v3"/>`),
  trackpad: svg(`<rect x="2.5" y="5" width="19" height="14" rx="3"/><path d="M2.5 15.5h19" stroke-opacity=".45"/>`),
  shield: svg(`<path d="M12 3l7.5 3v5.5c0 4.5-3.2 8.2-7.5 9.5-4.3-1.3-7.5-5-7.5-9.5V6z"/><path d="M9 12l2 2 4-4"/>`),
  refresh: svg(`<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3"/><path d="M19.5 4.5v4h-4"/>`),
  info: svg(`<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.5h.01" stroke-width="2.2"/>`),
};
