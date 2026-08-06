import { useEffect, useRef, useState } from "react";
import ChannelList from "../components/ChannelList";
import SearchBar from "../components/SearchBar";
import { api } from "../services/api";
import type { Channel, Group, Playlist } from "../services/types";

export default function Channels({ onPlayChannel }: { onPlayChannel: (ch: Channel) => void }) {
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [playlistId, setPlaylistId] = useState<number | null>(null);
  const [groups, setGroups] = useState<Group[]>([]);
  const [groupId, setGroupId] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [channels, setChannels] = useState<Channel[]>([]);
  const [busy, setBusy] = useState(false);
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
    api
      .listGroups(playlistId)
      .then((gs) => {
        setGroups(gs);
        setGroupId(gs.length > 0 ? gs[0].id : null);
      })
      .catch(console.error);
  }, [playlistId]);

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
          setChannels(await api.searchChannels(query.trim(), playlistId));
        }
      } catch (e) {
        console.error(e);
      } finally {
        setBusy(false);
      }
    }, 250);
    return () => window.clearTimeout(debounce.current);
  }, [query, groupId, playlistId]);

  return (
    <main className="page channels-page">
      <div className="sidebar">
        <select
          value={playlistId ?? ""}
          onChange={(e) => setPlaylistId(Number(e.target.value))}
        >
          {playlists.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <div className="group-list">
          <button
            className={groupId === null ? "group active" : "group"}
            onClick={() => setGroupId(null)}
          >
            All
          </button>
          {groups.map((g) => (
            <button
              key={g.id}
              className={groupId === g.id ? "group active" : "group"}
              onClick={() => setGroupId(g.id)}
            >
              {g.name}
            </button>
          ))}
        </div>
      </div>

      <div className="channel-pane">
        <SearchBar value={query} onChange={setQuery} />
        {busy && <p className="muted">Loading…</p>}
        <ChannelList channels={channels} onPlay={onPlayChannel} />
      </div>
    </main>
  );
}
