// Typed wrappers around the Tauri commands exposed by the Rust backend.
import { invoke } from "@tauri-apps/api/core";
import type {
  Channel,
  Episode,
  Group,
  ImportStats,
  Playlist,
  ServerInfo,
} from "./types";

// Logo URLs route through the LAN server's /api/logo proxy — same path the
// TV page uses — so hotlink-protected or plain-http provider logos load in
// the app too. Falls back to the raw URL if the server isn't up yet.
export function logoSrc(logoUrl: string | null, serverUrl: string | null): string | null {
  if (!logoUrl) return null;
  return serverUrl ? `${serverUrl}/api/logo?u=${encodeURIComponent(logoUrl)}` : logoUrl;
}

export const api = {
  importM3u: (source: string, name: string) =>
    invoke<ImportStats>("import_m3u", { source, name }),

  importXtream: (base: string, username: string, password: string, name: string) =>
    invoke<ImportStats>("import_xtream", { base, username, password, name }),

  listPlaylists: () => invoke<Playlist[]>("list_playlists"),

  deletePlaylist: (id: number) => invoke<void>("delete_playlist", { id }),

  listGroups: (playlistId: number, kind: string) =>
    invoke<Group[]>("list_groups", { playlistId, kind }),

  searchChannels: (
    query: string,
    playlistId: number | null,
    kind: string,
    limit = 500
  ) => invoke<Channel[]>("search_channels", { query, playlistId, kind, limit }),

  seriesEpisodes: (channelId: number) =>
    invoke<Episode[]>("series_episodes", { channelId }),

  channelsByGroup: (groupId: number) =>
    invoke<Channel[]>("channels_by_group", { groupId }),

  getChannel: (id: number) => invoke<Channel | null>("get_channel", { id }),

  getServerInfo: () => invoke<ServerInfo>("get_server_info"),

  // null clears the preference (automatic detection / default port).
  setServerPrefs: (ipOverride: string | null, port: number | null) =>
    invoke<ServerInfo>("set_server_prefs", { ipOverride, port }),
};
