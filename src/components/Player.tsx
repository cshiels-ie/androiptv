import { useEffect, useRef, useState } from "react";
import Hls from "hls.js";

// In-app player. Goes through the same /api/play endpoint as the TV
// page, so app and TV share one playback path (hls.js vs native-HLS
// fallback identical to the TV bundle).
export default function Player({
  src,
  autoplay = true,
  kind = "hls",
  casting = false,
}: {
  src: string;
  autoplay?: boolean;
  kind?: "hls" | "file";
  // True while the stream is being cast to a Chromecast: the phone's own
  // speakers shouldn't also play.
  casting?: boolean;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const hlsRef = useRef<Hls | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    setError(null);

    if (kind === "file") {
      // Native playback for VOD/series files — no hls.js, no live retry.
      video.src = src;
    } else {
      const native = video.canPlayType("application/vnd.apple.mpegurl");
      if (native && !Hls.isSupported()) {
        video.src = src;
      } else if (Hls.isSupported()) {
        // ffmpeg remux sessions need a few seconds to produce the first
        // playlist (503 "starting"); the default 1 retry gives up too early.
        const hls = new Hls({
          liveDurationInfinity: true,
          manifestLoadingMaxRetry: 30,
          manifestLoadingRetryDelay: 1500,
          manifestLoadingMaxRetryTimeout: 10000,
          manifestLoadingTimeOut: 15000,
        });
        hlsRef.current = hls;
        hls.on(Hls.Events.ERROR, (_e, data) => {
          if (data.fatal) {
            setError("Stream error — channel may be offline.");
            hls.destroy();
            hlsRef.current = null;
          }
        });
        hls.loadSource(src);
        hls.attachMedia(video);
        hls.on(Hls.Events.MANIFEST_PARSED, () => {
          if (autoplay) video.play().catch(() => {});
        });
      } else {
        video.src = src;
      }
    }

    return () => {
      hlsRef.current?.destroy();
      hlsRef.current = null;
      video.pause();
      video.removeAttribute("src");
      video.load();
    };
  }, [src, autoplay, kind]);

  useEffect(() => {
    if (casting) videoRef.current?.pause();
  }, [casting]);

  return (
    <div className="player-wrap">
      <video
        ref={videoRef}
        className="player-video"
        controls
        autoPlay={autoplay}
        playsInline
        onError={() =>
          setError("Playback error — the file may be offline or unsupported.")
        }
      />
      {error && <p className="err">{error}</p>}
    </div>
  );
}
