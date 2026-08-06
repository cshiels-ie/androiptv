import { useEffect, useState } from "react";
import Player from "../components/Player";
import type { Channel, PlayInfo, ServerInfo } from "../services/types";

// Playback overlay. Resolves /api/play on the local TV server so both
// the app and TV use the same proxy/transcode path.
export default function PlayerView({
  channel,
  serverInfo,
  onClose,
}: {
  channel: Channel;
  serverInfo: ServerInfo | null;
  onClose: () => void;
}) {
  const [playInfo, setPlayInfo] = useState<PlayInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!serverInfo) return;
    setError(null);
    setPlayInfo(null);
    fetch(`${serverInfo.url}/api/play/${channel.id}`)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((info: PlayInfo) => {
        if (info.error) throw new Error(info.error);
        setPlayInfo(info);
      })
      .catch((e) => setError(String(e)));
  }, [channel.id, serverInfo]);

  const src = playInfo ? `${serverInfo!.url}${playInfo.url}` : null;

  return (
    <div className="player-overlay" onClick={onClose}>
      <div className="player-card" onClick={(e) => e.stopPropagation()}>
        <div className="player-head">
          <strong>{channel.name}</strong>
          <button className="danger" onClick={onClose}>
            Close
          </button>
        </div>
        {error && <p className="err">{error}</p>}
        {!error && !src && <p className="muted">Starting stream…</p>}
        {!error && src && <Player key={src} src={src} />}
      </div>
    </div>
  );
}
