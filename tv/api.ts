// Typed helpers for the same-origin API of the embedded server.
export interface Group { id: number; playlist_id: number; name: string }
export interface Channel {
  id: number; playlist_id: number; group_id: number | null;
  name: string; url: string; logo_url: string | null;
  tvg_id: string | null; tvg_chno: number | null; kind: string;
}
export interface PlayInfo { kind: "hls" | "ts"; url: string; error?: string }

async function get<T>(path: string): Promise<T> {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`${path} → HTTP ${res.status}`);
  return res.json();
}

export const api = {
  groups: () => get<Group[]>("/api/groups"),
  channels: (groupId: number | null, query = "") => {
    const p = new URLSearchParams();
    if (groupId != null) p.set("group", String(groupId));
    if (query) p.set("q", query);
    return get<Channel[]>(`/api/channels?${p}`);
  },
  play: (channelId: number) => get<PlayInfo>(`/api/play/${channelId}`),
};
