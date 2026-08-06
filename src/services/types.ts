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
}

export interface ImportStats {
  channels: number;
  groups: number;
}

export interface ServerInfo {
  url: string;
  ips: string[];
  port: number;
}

export interface PlayInfo {
  kind: "hls" | "ts";
  url: string; // path relative to the TV server origin
  error?: string;
}
