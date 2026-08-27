import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Player from "../components/Player";
import type { Channel, PlayInfo, ServerInfo } from "../services/types";

// Cast state from the native Android plugin; null = plugin unavailable
// (desktop) → hide the button.
type CastState = {
  state: "disconnected" | "connecting" | "connected";
  device?: string;
};

// The Chromecast default receiver needs an explicit content type; guess
// it from the proxied URL's extension.
function mimeFor(url: string): string {
  const ext = (url.split("?")[0].split(".").pop() || "").toLowerCase();
  switch (ext) {
    case "mp4":
    case "m4v":
    case "mov":
      return "video/mp4";
    case "mkv":
      return "video/x-matroska";
    case "webm":
      return "video/webm";
    case "ts":
    case "m2ts":
      return "video/mp2t";
    case "m3u8":
      return "application/x-mpegurl";
    default:
      return "video/mp4";
  }
}

function CastIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor" aria-hidden="true">
      <path d="M1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.92-11-11-11zm20-7H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2z" />
    </svg>
  );
}

// Playback overlay. Resolves /api/play on the local TV server and plays the
// single recommended path — no backend picker: the server already chooses
// the right one per content (HLS → proxy, live TS → ffmpeg remux, native
// VOD files → direct panel URL, mkv/avi VOD → ffmpeg remux).
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
  const [cast, setCast] = useState<CastState | null>(null);
  const [castError, setCastError] = useState<string | null>(null);

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

  // Surface ffmpeg session failures instead of letting hls.js retry an
  // eternal 503 for ~45s: peek at the remux manifest once — a 503 body
  // carries the ffmpeg stderr ("ffmpeg exited before the stream was
  // remuxed: …"), which is exactly what the user needs to know.
  useEffect(() => {
    if (!serverInfo || !playInfo) return;
    if (playInfo.kind !== "ts" || !playInfo.ts) return;
    let cancelled = false;
    fetch(`${serverInfo.url}${playInfo.ts}`)
      .then(async (r) => {
        if (r.ok || cancelled) return;
        const body = await r.json().catch(() => null);
        if (!cancelled && body?.error && body.error !== "starting") {
          setError(`Server: ${body.error}${body.stderr ? `\n${body.stderr}` : ""}`);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [serverInfo, playInfo]);

  // Cast availability probe: desktop has no plugin, so the invoke rejects
  // and the button stays hidden.
  useEffect(() => {
    invoke("plugin:cast|is-available")
      .then(() => setCast({ state: "disconnected" }))
      .catch(() => setCast(null));
  }, []);

  const castSupported = cast !== null;
  useEffect(() => {
    if (!castSupported) return;
    const t = setInterval(() => {
      invoke<{ state: CastState["state"]; device?: string }>("plugin:cast|state")
        .then((s) => setCast({ state: s.state, device: s.device }))
        .catch(() => {});
    }, 2000);
    return () => clearInterval(t);
  }, [castSupported]);

  async function toggleCast() {
    if (!playInfo || !proxySrc) return;
    if (cast?.state === "connected") {
      await invoke("plugin:cast|disconnect").catch(() => {});
      setCast({ state: "disconnected" });
      return;
    }
    setCastError(null);
    setCast({ state: "connecting" });
    try {
      await invoke("plugin:cast|connect");
      await invoke("plugin:cast|load", {
        // Files cast straight from the panel (same as playback) so a long
        // movie isn't cut short by the proxy's request timeout.
        url: playInfo.kind === "file" ? (playInfo.direct ?? proxySrc) : proxySrc,
        title: channel.name,
        contentType: mimeFor(proxySrc),
        streamType: playInfo.kind === "file" ? "buffered" : "live",
      });
    } catch (e) {
      setCastError(String(e));
      setCast({ state: "disconnected" });
    }
  }

  const proxySrc = playInfo ? `${serverInfo!.url}${playInfo.url}` : null;

  // Single automatic path: native files play the panel URL directly; HLS and
  // ffmpeg-remuxed streams play through the same-origin proxy with hls.js.
  let effective: { kind: "hls" | "file"; src: string } | null = null;
  if (playInfo && proxySrc) {
    effective =
      playInfo.kind === "file"
        ? { kind: "file", src: playInfo.direct ?? proxySrc }
        : { kind: "hls", src: proxySrc };
  }

  return (
    <div className="player-overlay" onClick={onClose}>
      <div className="player-card" onClick={(e) => e.stopPropagation()}>
        <div className="player-head">
          <strong>{channel.name}</strong>
          {playInfo && castSupported && (
            <button
              className={`cast-button${cast!.state === "connected" ? " active" : ""}`}
              onClick={toggleCast}
              disabled={cast!.state === "connecting"}
              title={
                cast!.state === "connected"
                  ? `Stop casting to ${cast!.device || "device"}`
                  : "Cast to TV"
              }
            >
              <CastIcon />
              {cast!.state === "connected"
                ? "Casting"
                : cast!.state === "connecting"
                  ? "Connecting…"
                  : "Cast"}
            </button>
          )}
          <button className="danger" onClick={onClose}>
            Close
          </button>
        </div>
        {error && <p className="err">{error}</p>}
        {castError && <p className="err">{castError}</p>}
        {!error && !effective && <p className="muted">Starting stream…</p>}
        {!error && effective && (
          <Player
            key={effective.src}
            src={effective.src}
            kind={effective.kind}
            casting={cast?.state === "connected"}
          />
        )}
      </div>
    </div>
  );
}
