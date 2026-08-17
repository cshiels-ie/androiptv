import { useEffect, useRef, useState } from "react";
import ChannelList from "../components/ChannelList";
import SearchBar from "../components/SearchBar";
import SeriesView from "../components/SeriesView";
import { HamburgerIcon } from "../components/icons";
import { api } from "../services/api";
import type { Channel, Episode, Group, Playlist } from "../services/types";

type Kind = "live" | "vod" | "series";

export default function Channels({
  onPlayChannel,
  onPlayEpisode,
  serverUrl,
}: {
  onPlayChannel: (ch: Channel) => void;
  onPlayEpisode: (series: Channel, ep: Episode) => void;
  serverUrl: string | null;
}) {
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [playlistId, setPlaylistId] = useState<number | null>(null);
  const [groups, setGroups] = useState<Group[]>([]);
  const [groupId, setGroupId] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [channels, setChannels] = useState<Channel[]>([]);
  const [busy, setBusy] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [kind, setKind] = useState<Kind>("live");
  const [series, setSeries] = useState<Channel | null>(null);
  const debounce = useRef<number | undefined>(undefined);

  useEffect(() => {
    api
      .listPlaylists()
      .then((ps) => {
        setPlaylists(ps);
        if (ps.length > 0 && playlistId === null) setPlaylistId(ps[0].id);
      })
      .catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (playlistId === null) return;
    setSeries(null);
    setQuery("");
    api
      .listGroups(playlistId, kind)
      .then((gs) => {
        setGroups(gs);
        setGroupId(gs.length > 0 ? gs[0].id : null);
      })
      .catch(console.error);
  }, [playlistId, kind]);

  // Debounced search; falls back to group browsing when query is empty.
  useEffect(() => {
    if (playlistId === null) return;
    window.clearTimeout(debounce.current);
    setBusy(true);
    debounce.current = window.setTimeout(async () => {
      try {
        if (query.trim().length === 0) {
          if (groupId !== null) {
            setChannels(await api.channelsByGroup(groupId));
          } else {
            setChannels([]);
          }
        } else {
          setChannels(await api.searchChannels(query.trim(), playlistId, kind));
        }
      } catch (e) {
        console.error(e);
      } finally {
        setBusy(false);
      }
    }, 250);
    return () => window.clearTimeout(debounce.current);
  }, [query, groupId, playlistId, kind]);

  const pickGroup = (id: number | null) => {
    setGroupId(id);
    setSidebarOpen(false);
  };

  if (series) {
    return (
      <main className="page channels-page">
        <div className="channel-pane">
          <SeriesView
            series={series}
            onBack={() => setSeries(null)}
            onPlayEpisode={onPlayEpisode}
            serverUrl={serverUrl}
          />
        </div>
      </main>
    );
  }

  const placeholder =
    kind === "live"
      ? "Search channels…"
      : kind === "vod"
        ? "Search movies…"
        : "Search series…";
  const emptyLabel =
    kind === "live"
      ? "No channels"
      : kind === "vod"
        ? "No movies — import an Xtream playlist"
        : "No series — import an Xtream playlist";

  return (
    <main
      className={`page channels-page${sidebarOpen ? "" : " sidebar-collapsed"}`}
    >
      <div className="sidebar">
        <div className="group-list">
          <button
            className={groupId === null ? "group active" : "group"}
            onClick={() => pickGroup(null)}
          >
            All
          </button>
          {groups.map((g) => (
            <button
              key={g.id}
              className={groupId === g.id ? "group active" : "group"}
              onClick={() => pickGroup(g.id)}
            >
              {g.name}
            </button>
          ))}
        </div>
      </div>

      <div className="channel-pane">
        <div className="pane-header">
          {!sidebarOpen && (
            <button
              className="hamburger"
              aria-label="Show sidebar"
              onClick={() => setSidebarOpen(true)}
            >
              <HamburgerIcon />
            </button>
          )}
          <select
            className="pane-playlist"
            value={playlistId ?? ""}
            onChange={(e) => setPlaylistId(Number(e.target.value))}
          >
            {playlists.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </div>
        <div className="pane-header">
          <div className="segmented">
            <button
              className={kind === "live" ? "seg active" : "seg"}
              onClick={() => setKind("live")}
            >
              Live
            </button>
            <button
              className={kind === "vod" ? "seg active" : "seg"}
              onClick={() => setKind("vod")}
            >
              VOD
            </button>
            <button
              className={kind === "series" ? "seg active" : "seg"}
              onClick={() => setKind("series")}
            >
              Series
            </button>
          </div>
          <SearchBar
            value={query}
            onChange={setQuery}
            placeholder={placeholder}
          />
        </div>
        {busy && <p className="muted">Loading…</p>}
        <ChannelList
          channels={channels}
          onPlay={kind === "series" ? (ch) => setSeries(ch) : onPlayChannel}
          emptyLabel={emptyLabel}
          serverUrl={serverUrl}
        />
      </div>
    </main>
  );
}
