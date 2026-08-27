// TV browser entry point (vanilla TS, no framework). Hash router:
//   #/channels            — Live: group nav + channel grid
//   #/movies              — Movies (VOD): group nav + grid
//   #/series              — TV Shows: series grid
//   #/series/<id>         — episodes of one series
//   #/play/<channelId>    — play a live/VOD channel
//   #/play/episode/<id>   — play one episode
// D-pad / arrow-key navigation with focus management.
import "./styles.css";
import { api } from "./api";
import type { Channel, Episode, Kind } from "./api";
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

// Current tab. The trailing "#/channels" is just so location.hash always
// has a route to route() on.
let tab: Kind = "live";

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

// One card builder for renderChannels and loadChannels. Channels without
// a logo (or whose logo fetch fails) get the <img> hidden entirely — a
// hidden-but-laid-out image leaves a gaping hole, and a broken one shows
// the browser's broken-image glyph.
function buildCard(ch: Channel, onOpen: () => void): HTMLButtonElement {
  const card = el("button", "card");
  const img = el("img");
  if (ch.logo_url) {
    img.src = `/api/logo?u=${encodeURIComponent(ch.logo_url)}`;
    img.onerror = () => { img.style.display = "none"; };
  } else {
    img.style.display = "none";
  }
  const text = el("span");
  text.appendChild(el("span", "name", ch.name));
  if (ch.tvg_chno != null) text.appendChild(el("span", "chno", `CH ${ch.tvg_chno}`));
  card.append(img, text);
  card.onclick = onOpen;
  return card;
}

function episodeCard(ep: Episode): HTMLButtonElement {
  const card = el("button", "card");
  const img = el("img");
  if (ep.logo_url) {
    img.src = `/api/logo?u=${encodeURIComponent(ep.logo_url)}`;
    img.onerror = () => { img.style.display = "none"; };
  } else {
    img.style.display = "none";
  }
  const text = el("span");
  text.appendChild(
    el("span", "name", `S${ep.season}E${ep.episode_num} · ${ep.title || `Episode ${ep.episode_num}`}`),
  );
  card.append(img, text);
  card.onclick = () => (location.hash = `#/play/episode/${ep.id}`);
  return card;
}

// ---------- routing ----------
async function route() {
  cleanup?.();
  cleanup = null;
  const hash = location.hash || "#/channels";
  let m = hash.match(/^#\/play\/episode\/(\d+)/);
  if (m) {
    const episodeId = Number(m[1]);
    return renderPlayer(() => api.playEpisode(episodeId));
  }
  m = hash.match(/^#\/play\/(\d+)/);
  if (m) {
    const channelId = Number(m[1]);
    return renderPlayer(() => api.play(channelId));
  }
  m = hash.match(/^#\/series\/(\d+)/);
  if (m) return renderEpisodes(Number(m[1]));
  if (/^#\/series/.test(hash)) return renderSeriesList();
  if (/^#\/movies/.test(hash)) return renderChannels("vod");
  return renderChannels("live");
}

// ---------- header tabs ----------
function renderTabs(current: Kind) {
  const tabs: Array<[Kind, string, string]> = [
    ["live", "Live", "#/channels"],
    ["vod", "Movies", "#/movies"],
    ["series", "TV Shows", "#/series"],
  ];
  const nav = el("nav", "tabs");
  nav.id = "tabs";
  for (const [kind, label, href] of tabs) {
    const b = el("button", kind === current ? "tab active" : "tab", label);
    b.onclick = () => { location.hash = href; };
    nav.appendChild(b);
  }
  const existing = document.getElementById("tabs");
  if (existing) {
    existing.replaceWith(nav);
  } else {
    // Insert between the title and the actions row.
    hdr.insertBefore(nav, actions);
  }
}

// ---------- channels (live / movies) ----------
async function renderChannels(kind: Kind) {
  tab = kind;
  renderTabs(kind);
  actions.replaceChildren();
  const back = el("button", "link", "Refresh");
  back.onclick = () => loadChannels(true, kind);
  actions.appendChild(back);

  try {
    [groups, channels] = await Promise.all([api.groups(kind), api.channels(activeGroup, "", kind)]);
  } catch (e) {
    main.replaceChildren();
    main.appendChild(el("p", "muted", "Server unreachable — is the host app running?"));
    ftr.textContent = String(e);
    return;
  }

  ftr.textContent = `${channels.length} ${kind === "live" ? "channels" : kind === "vod" ? "movies" : "shows"} · ${groups.length} groups · running on this device's local network`;

  const gbar = el("div", "groups");
  const allBtn = el("button", activeGroup === null ? "active" : "", "All");
  allBtn.onclick = () => { activeGroup = null; loadChannels(true, kind); };
  gbar.appendChild(allBtn);
  for (const g of groups) {
    const b = el("button", g.id === activeGroup ? "active" : "", g.name);
    b.onclick = () => { activeGroup = g.id; loadChannels(true, kind); };
    gbar.appendChild(b);
  }

  const grid = el("div", "grid");
  for (const ch of channels) {
    grid.appendChild(buildCard(ch, () => (location.hash = `#/play/${ch.id}`)));
  }

  main.replaceChildren(gbar, grid);
  focusIndex(0);
}

async function loadChannels(restoreFocus: boolean, kind: Kind) {
  try {
    channels = await api.channels(activeGroup, "", kind);
    ftr.textContent = `${channels.length} ${kind === "vod" ? "movies" : "channels"}`;
  } catch (e) {
    toast(String(e));
    return;
  }
  const grid = main.querySelector(".grid")!;
  grid.replaceChildren();
  for (const ch of channels) {
    grid.appendChild(buildCard(ch, () => (location.hash = `#/play/${ch.id}`)));
  }
  if (restoreFocus) focusIndex(0);
}

// ---------- series ----------
async function renderSeriesList() {
  tab = "series";
  renderTabs("series");
  actions.replaceChildren();

  let groups: Awaited<ReturnType<typeof api.groups>>;
  let all: Channel[];
  try {
    [groups, all] = await Promise.all([
      api.groups("series"),
      api.channels(null, "", "series"),
    ]);
  } catch (e) {
    main.replaceChildren();
    main.appendChild(el("p", "muted", "Server unreachable — is the host app running?"));
    ftr.textContent = String(e);
    return;
  }
  ftr.textContent = `${all.length} shows · running on this device's local network`;

  const gbar = el("div", "groups");
  const allBtn = el("button", activeGroup === null ? "active" : "", "All");
  allBtn.onclick = () => { activeGroup = null; loadSeries(true); };
  gbar.appendChild(allBtn);
  for (const g of groups) {
    const b = el("button", g.id === activeGroup ? "active" : "", g.name);
    b.onclick = () => { activeGroup = g.id; loadSeries(true); };
    gbar.appendChild(b);
  }

  const grid = el("div", "grid");
  for (const ch of all) {
    grid.appendChild(buildCard(ch, () => (location.hash = `#/series/${ch.id}`)));
  }

  main.replaceChildren(gbar, grid);
  focusIndex(0);
}

async function loadSeries(restoreFocus: boolean) {
  try {
    channels = await api.channels(activeGroup, "", "series");
    ftr.textContent = `${channels.length} shows`;
  } catch (e) {
    toast(String(e));
    return;
  }
  const grid = main.querySelector(".grid")!;
  grid.replaceChildren();
  for (const ch of channels) {
    grid.appendChild(buildCard(ch, () => (location.hash = `#/series/${ch.id}`)));
  }
  if (restoreFocus) focusIndex(0);
}

async function renderEpisodes(channelId: number) {
  tab = "series";
  renderTabs("series");
  actions.replaceChildren();
  const back = el("button", "", "◀ Shows");
  back.onclick = () => (location.hash = "#/series");
  actions.appendChild(back);

  let eps: Episode[];
  try {
    eps = await api.seriesEpisodes(channelId);
  } catch (e) {
    main.replaceChildren(el("p", "muted", "Couldn't load episodes."));
    ftr.textContent = String(e);
    return;
  }

  ftr.textContent = `${eps.length} episodes`;
  if (!eps.length) {
    main.replaceChildren(el("p", "muted", "No episodes for this show yet."));
    return;
  }

  const grid = el("div", "grid");
  for (const ep of eps) grid.appendChild(episodeCard(ep));
  main.replaceChildren(grid);
  focusIndex(0);
}

// ---------- player ----------
async function renderPlayer(getInfo: () => Promise<Awaited<ReturnType<typeof api.play>>>) {
  renderTabs(tab);
  actions.replaceChildren();
  const back = el("button", "", "◀ Back");
  back.onclick = () => {
    if (tab === "series") location.hash = "#/series";
    else if (tab === "vod") location.hash = "#/movies";
    else location.hash = "#/channels";
  };
  actions.appendChild(back);

  let info: Awaited<ReturnType<typeof api.play>>;
  try {
    info = await getInfo();
  } catch (e) {
    main.replaceChildren(el("p", "muted", `Couldn't start: ${String(e)}`));
    return;
  }

  const wrap = el("div", "player-wrap");
  wrap.id = "player-wrap";
  const now = el("div");
  now.id = "now-info";
  const name = el("span", "name", info && info.url ? "Loading…" : "Playback unavailable");
  now.appendChild(name);
  const video = el("video");
  video.autoplay = true;
  video.controls = true;
  video.setAttribute("playsinline", "");
  wrap.append(now, video);
  main.replaceChildren(wrap);

  if (!info || info.error || !info.url) {
    name.textContent = info?.error || "Playback unavailable";
    if (info?.error) toast(info.error);
    return;
  }

  // Give hls.js a stable src (it resolves .m3u8 into segment requests itself).
  const src = info.url;
  name.textContent = "Buffering…";
  cleanup = attachPlayer(video, src, (msg) => {
    name.textContent = msg;
    toast(msg);
  });
  video.addEventListener("playing", () => (name.textContent = "▶ Playing"));
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
      if (location.hash.startsWith("#/play/")) {
        location.hash = tab === "series" ? "#/series" : tab === "vod" ? "#/movies" : "#/channels";
      } else if (location.hash.startsWith("#/series/")) {
        location.hash = "#/series";
      }
      break;
    }
  }
});

window.addEventListener("hashchange", route);
route().catch((e) => {
  main.replaceChildren(el("p", "muted", String(e)));
});
