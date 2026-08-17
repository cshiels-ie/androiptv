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
};
