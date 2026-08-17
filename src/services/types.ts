// Shared domain types mirrored from the Rust backend (serde).

export interface Playlist {
  id: number;
  name: string;
  source_type: "m3u" | "xtream";
  source_url: string;
  xtream_base: string | null;
  created_at: number;
}

export interface Group {
  id: number;
  playlist_id: number;
  name: string;
  kind: string;
}

export interface Channel {
  id: number;
  playlist_id: number;
  group_id: number | null;
  name: string;
  url: string;
  logo_url: string | null;
  tvg_id: string | null;
  tvg_chno: number | null;
  kind: string;
  remote_id: string | null;
}

export interface ImportStats {
  channels: number;
  groups: number;
  vod: number;
  series: number;
}

// One playable file of a VOD/series entry (mirrored from the Rust backend).
export interface Episode {
  id: number;
  channel_id: number;
  season: number;
  episode_num: number;
  title: string;
  url: string;
  logo_url: string | null;
}

export interface ServerInfo {
  url: string;
  ips: string[];
  port: number;
}

export interface PlayInfo {
  kind: "hls" | "ts" | "file";
  url: string; // path relative to the TV server origin
  error?: string;
}
