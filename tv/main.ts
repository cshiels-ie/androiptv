// TV browser entry point (vanilla TS, no framework). Hash router:
//   #/channels            — group nav + channel grid
//   #/play/<channelId>    — video player
// D-pad / arrow-key navigation with focus management.
import "./styles.css";
import { api } from "./api";
import type { Channel } from "./api";
import { attachPlayer } from "./player";

const hdr = document.getElementById("hdr")!;
const actions = document.getElementById("hdr-actions")!;
const main = document.getElementById("main")!;
const ftr = document.getElementById("ftr")!;

let groups: Awaited<ReturnType<typeof api.groups>> = [];
let channels: Channel[] = [];
let activeGroup: number | null = null;
let selectedId = 0;
let cleanup: (() => void) | null = null;

// ---------- small DOM helpers ----------
function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

function toast(msg: string) {
  const t = el("div", "toast", msg);
  document.body.appendChild(t);
  setTimeout(() => t.remove(), 5000);
}

function focusable(): HTMLElement[] {
  return Array.from(main.querySelectorAll<HTMLElement>("button, .card"));
}

function focusIndex(i: number) {
  const items = focusable();
  if (items.length === 0) return;
  selectedId = (i + items.length) % items.length;
  items[selectedId].focus();
  items[selectedId].scrollIntoView({ block: "nearest" });
}

// ---------- routing ----------
async function route() {
  cleanup?.();
  cleanup = null;
  const hash = location.hash || "#/channels";
  const m = hash.match(/^#\/play\/(\d+)/);
  if (m) return renderPlayer(Number(m[1]));
  await renderChannels();
}

// ---------- channels ----------
async function renderChannels() {
  hdr.querySelector("h1")!.textContent = "AndroIPTV — Channels";
  actions.replaceChildren();
  const back = el("button", "link", "Refresh");
  back.onclick = () => loadChannels(true);
  actions.appendChild(back);

  try {
    [groups, channels] = await Promise.all([api.groups(), api.channels(activeGroup)]);
  } catch (e) {
    main.replaceChildren();
    main.appendChild(el("p", "muted", "Server unreachable — is the host app running?"));
    ftr.textContent = String(e);
    return;
  }

  ftr.textContent = `${channels.length} channels · ${groups.length} groups · running on this device's local network`;

  const gbar = el("div", "groups");
  const allBtn = el("button", activeGroup === null ? "active" : "", "All");
  allBtn.onclick = () => { activeGroup = null; loadChannels(true); };
  gbar.appendChild(allBtn);
  for (const g of groups) {
    const b = el("button", g.id === activeGroup ? "active" : "", g.name);
    b.onclick = () => { activeGroup = g.id; loadChannels(true); };
    gbar.appendChild(b);
  }

  const grid = el("div", "grid");
  for (const ch of channels) {
    const card = el("button", "card");
    const img = el("img");
    if (ch.logo_url) {
      img.src = `/api/logo?u=${encodeURIComponent(ch.logo_url)}`;
    } else {
      img.style.visibility = "hidden";
    }
    const name = el("span", "name", ch.name);
    card.appendChild(img);
    const text = el("span");
    text.appendChild(name);
    if (ch.tvg_chno != null) text.appendChild(el("span", "chno", `CH ${ch.tvg_chno}`));
    card.appendChild(text);
    card.onclick = () => (location.hash = `#/play/${ch.id}`);
    grid.appendChild(card);
  }

  main.replaceChildren(gbar, grid);
  focusIndex(0);
}

async function loadChannels(restoreFocus: boolean) {
  try {
    channels = await api.channels(activeGroup);
    ftr.textContent = `${channels.length} channels`;
  } catch (e) {
    toast(String(e));
    return;
  }
  const grid = main.querySelector(".grid")!;
  grid.replaceChildren();
  for (const ch of channels) {
    const card = el("button", "card");
    // …build cards identically to renderChannels (kept local on purpose)
    const img = el("img");
    if (ch.logo_url) img.src = `/api/logo?u=${encodeURIComponent(ch.logo_url)}`;
    else img.style.visibility = "hidden";
    const text = el("span");
    text.appendChild(el("span", "name", ch.name));
    if (ch.tvg_chno != null) text.appendChild(el("span", "chno", `CH ${ch.tvg_chno}`));
    card.append(img, text);
    card.onclick = () => (location.hash = `#/play/${ch.id}`);
    grid.appendChild(card);
  }
  if (restoreFocus) focusIndex(0);
}

// ---------- player ----------
async function renderPlayer(channelId: number) {
  actions.replaceChildren();
  const back = el("button", "", "◀ Back");
  back.onclick = () => (location.hash = "#/channels");
  actions.appendChild(back);

  let info: Awaited<ReturnType<typeof api.play>>;
  try {
    info = await api.play(channelId);
  } catch (e) {
    main.replaceChildren(el("p", "muted", `Channel ${channelId} not found.`));
    return;
  }

  const wrap = el("div", "player-wrap");
  wrap.id = "player-wrap";
  const now = el("div");
  now.id = "now-info";
  const name = el("span", "name", info.error ? "Playback unavailable" : `▶ ${info.url ? "Loading…" : ""}`);
  now.appendChild(name);
  const video = el("video");
  video.autoplay = true;
  video.controls = true;
  video.setAttribute("playsinline", "");
  wrap.append(now, video);
  main.replaceChildren(wrap);

  if (info.error) {
    name.textContent = info.error;
    toast(info.error);
    return;
  }

  // Give hls.js a stable src (it resolves .m3u8 into segment requests itself).
  const src = info.url;
  name.textContent = "Buffering…";
  cleanup = attachPlayer(video, src, (msg) => {
    name.textContent = msg;
    toast(msg);
  });
  video.addEventListener("playing", () => (name.textContent = "▶ LIVE"));
}

// ---------- keyboard / remote (D-pad) ----------
document.addEventListener("keydown", (e) => {
  const items = focusable();
  if (items.length === 0) return;
  const grid = main.querySelector(".grid");
  const cols = grid ? Math.max(1, Math.floor(grid.clientWidth / 272)) : 1;

  switch (e.key) {
    case "ArrowRight": case "Right": e.preventDefault(); focusIndex(selectedId + 1); break;
    case "ArrowLeft": case "Left": e.preventDefault(); focusIndex(selectedId - 1); break;
    case "ArrowDown": case "Down": e.preventDefault(); focusIndex(selectedId + cols); break;
    case "ArrowUp": case "Up": e.preventDefault(); focusIndex(selectedId - cols); break;
    case "Backspace": case "Escape": {
      if (location.hash.startsWith("#/play/")) location.hash = "#/channels";
      break;
    }
  }
});

window.addEventListener("hashchange", route);
route().catch((e) => {
  main.replaceChildren(el("p", "muted", String(e)));
});
