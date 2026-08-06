import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../services/api";
import type { ImportStats, Playlist } from "../services/types";

type ImportMode = "m3u-url" | "m3u-file" | "xtream";

export default function PlaylistManager({ onImported }: { onImported: () => void }) {
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [mode, setMode] = useState<ImportMode>("m3u-url");
  const [name, setName] = useState("");
  const [source, setSource] = useState("");
  const [xtreamBase, setXtreamBase] = useState("");
  const [xtreamUser, setXtreamUser] = useState("");
  const [xtreamPass, setXtreamPass] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastStats, setLastStats] = useState<ImportStats | null>(null);

  const refresh = () => api.listPlaylists().then(setPlaylists).catch(setError);
  useEffect(() => {
    refresh();
  }, []);

  const pickFile = async () => {
    const file = await open({
      multiple: false,
      filters: [{ name: "Playlists", extensions: ["m3u", "m3u8", "txt"] }],
    });
    if (typeof file === "string") setSource(file);
  };

  const doImport = async () => {
    setError(null);
    setLastStats(null);
    setBusy(true);
    try {
      let stats: ImportStats;
      if (mode === "xtream") {
        if (!xtreamBase || !xtreamUser || !xtreamPass) {
          throw new Error("Base URL, username and password are required");
        }
        stats = await api.importXtream(xtreamBase, xtreamUser, xtreamPass, name || xtreamBase);
      } else {
        if (!source) throw new Error(mode === "m3u-file" ? "Pick a file first" : "Enter a playlist URL");
        stats = await api.importM3u(source, name || source);
      }
      setLastStats(stats);
      setSource("");
      setName("");
      await refresh();
      onImported();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel">
      <h2>Playlists</h2>

      <div className="import-card">
        <div className="segmented">
          {(["m3u-url", "m3u-file", "xtream"] as ImportMode[]).map((m) => (
            <button
              key={m}
              className={mode === m ? "seg active" : "seg"}
              onClick={() => setMode(m)}
            >
              {m === "m3u-url" ? "M3U URL" : m === "m3u-file" ? "M3U File" : "Xtream Codes"}
            </button>
          ))}
        </div>

        <input
          placeholder="Name (optional)"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />

        {mode === "m3u-url" && (
          <input
            placeholder="https://example.com/playlist.m3u8"
            value={source}
            onChange={(e) => setSource(e.target.value)}
          />
        )}
        {mode === "m3u-file" && (
          <div className="row">
            <input
              placeholder="Local playlist file path"
              value={source}
              onChange={(e) => setSource(e.target.value)}
            />
            <button onClick={pickFile}>Browse…</button>
          </div>
        )}
        {mode === "xtream" && (
          <>
            <input
              placeholder="Server base URL — http://host:port"
              value={xtreamBase}
              onChange={(e) => setXtreamBase(e.target.value)}
            />
            <div className="row">
              <input placeholder="Username" value={xtreamUser} onChange={(e) => setXtreamUser(e.target.value)} />
              <input placeholder="Password" type="password" value={xtreamPass} onChange={(e) => setXtreamPass(e.target.value)} />
            </div>
          </>
        )}

        <button className="primary" disabled={busy} onClick={doImport}>
          {busy ? "Importing…" : "Import"}
        </button>

        {lastStats && (
          <p className="ok">
            Imported {lastStats.channels} channels in {lastStats.groups} groups.
          </p>
        )}
        {error && <p className="err">{error}</p>}
      </div>

      <ul className="playlist-list">
        {playlists.map((p) => (
          <li key={p.id}>
            <div>
              <strong>{p.name}</strong>
              <span className="muted">
                {p.source_type} · {p.source_url}
              </span>
            </div>
            <button
              className="danger"
              onClick={async () => {
                await api.deletePlaylist(p.id);
                refresh();
                onImported();
              }}
            >
              Delete
            </button>
          </li>
        ))}
        {playlists.length === 0 && <li className="muted">No playlists yet — import one above.</li>}
      </ul>
    </div>
  );
}
