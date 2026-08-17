import { useEffect, useState } from "react";
import Player from "../components/Player";
import type { Channel, PlayInfo, ServerInfo } from "../services/types";

// Playback backends the player can be forced to use, in order. Auto keeps
// the server-recommended kind; the rest exist as fallbacks for panels
// where the default path fails (no ffmpeg, blocked proxy, odd codecs).
const PLAY_MODES = [
  ["auto", "Auto"],
  ["hls", "HLS (hls.js)"],
  ["native", "Native video"],
  ["ts", "Remux (ffmpeg)"],
  ["direct", "Direct URL"],
] as const;
type PlayMode = (typeof PLAY_MODES)[number][0];

// Playback overlay. Resolves /api/play on the local TV server so both
// the app and TV use the same proxy/transcode path.
export default function PlayerView({
  channel,
  episodeId,
  serverInfo,
  onClose,
}: {
  channel: Channel;
  episodeId?: number;
  serverInfo: ServerInfo | null;
  onClose: () => void;
}) {
  const [playInfo, setPlayInfo] = useState<PlayInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The chosen backend persists across streams (localStorage) so a panel
  // that only works with one method needs no re-picking.
  const [mode, setMode] = useState<PlayMode>(() => {
    const saved = localStorage.getItem("androiptv.playbackMode");
    return PLAY_MODES.some(([m]) => m === saved) ? (saved as PlayMode) : "auto";
  });

  useEffect(() => {
    if (!serverInfo) return;
    setError(null);
    setPlayInfo(null);
    const url =
      episodeId != null
        ? `${serverInfo.url}/api/play/episode/${episodeId}`
        : `${serverInfo.url}/api/play/${channel.id}`;
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((info: PlayInfo) => {
        if (info.error) throw new Error(info.error);
        setPlayInfo(info);
      })
      .catch((e) => setError(String(e)));
  }, [channel.id, episodeId, serverInfo]);

  const proxySrc = playInfo ? `${serverInfo!.url}${playInfo.url}` : null;

  // Resolve the chosen mode into concrete (kind, src). A forced mode whose
  // URL the server didn't provide falls back to Auto.
  let effective: { kind: "hls" | "file"; src: string } | null = null;
  if (playInfo && proxySrc) {
    const auto =
      playInfo.kind === "file"
        ? { kind: "file" as const, src: proxySrc }
        : { kind: "hls" as const, src: proxySrc };
    if (mode === "native") effective = { kind: "file", src: proxySrc };
    else if (mode === "direct")
      effective = playInfo.direct
        ? { kind: "file", src: playInfo.direct }
        : auto;
    else if (mode === "ts")
      effective = playInfo.ts
        ? { kind: "hls", src: `${serverInfo!.url}${playInfo.ts}` }
        : auto;
    else if (mode === "hls") effective = { kind: "hls", src: proxySrc };
    else effective = auto;
  }

  return (
    <div className="player-overlay" onClick={onClose}>
      <div className="player-card" onClick={(e) => e.stopPropagation()}>
        <div className="player-head">
          <strong>{channel.name}</strong>
          {playInfo && (
            <select
              className="playback-switch"
              value={mode}
              onChange={(e) => {
                const next = e.target.value as PlayMode;
                setMode(next);
                localStorage.setItem("androiptv.playbackMode", next);
              }}
              title="Playback method — try another if this one fails"
            >
              {PLAY_MODES.map(([m, label]) => (
                <option key={m} value={m}>
                  {label}
                </option>
              ))}
            </select>
          )}
          <button className="danger" onClick={onClose}>
            Close
          </button>
        </div>
        {error && <p className="err">{error}</p>}
        {!error && !effective && <p className="muted">Starting stream…</p>}
        {!error && effective && (
          <Player key={effective.src} src={effective.src} kind={effective.kind} />
        )}
      </div>
    </div>
  );
}
