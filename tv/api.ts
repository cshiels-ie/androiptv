// Typed helpers for the same-origin API of the embedded server.
export interface Group { id: number; playlist_id: number; name: string }
export interface Channel {
  id: number; playlist_id: number; group_id: number | null;
  name: string; url: string; logo_url: string | null;
  tvg_id: string | null; tvg_chno: number | null; kind: string;
}
export interface Episode {
  id: number; channel_id: number; season: number;
  episode_num: number; title: string; url: string; logo_url: string | null;
}
export interface PlayInfo { kind: "hls" | "ts"; url: string; error?: string }

async function get<T>(path: string): Promise<T> {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`${path} → HTTP ${res.status}`);
  return res.json();
}

export type Kind = "live" | "vod" | "series";

export const api = {
  groups: (kind: Kind) => get<Group[]>(`/api/groups?kind=${kind}`),
  channels: (groupId: number | null, query = "", kind: Kind = "live") => {
    const p = new URLSearchParams();
    p.set("kind", kind);
    if (groupId != null) p.set("group", String(groupId));
    if (query) p.set("q", query);
    return get<Channel[]>(`/api/channels?${p}`);
  },
  seriesEpisodes: (channelId: number) =>
    get<Episode[]>(`/api/series/${channelId}/episodes`),
  play: (channelId: number) => get<PlayInfo>(`/api/play/${channelId}`),
  playEpisode: (episodeId: number) => get<PlayInfo>(`/api/play/episode/${episodeId}`),
};
