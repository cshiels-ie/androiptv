import { useEffect, useRef, useState } from "react";
import Hls from "hls.js";

// In-app player. Goes through the same /api/play endpoint as the TV
// page, so app and TV share one playback path (hls.js vs native-HLS
// fallback identical to the TV bundle).
export default function Player({
  src,
  autoplay = true,
}: {
  src: string;
  autoplay?: boolean;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const hlsRef = useRef<Hls | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    setError(null);

    const native = video.canPlayType("application/vnd.apple.mpegurl");
    if (native && !Hls.isSupported()) {
      video.src = src;
    } else if (Hls.isSupported()) {
      const hls = new Hls({ liveDurationInfinity: true });
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

    return () => {
      hlsRef.current?.destroy();
      hlsRef.current = null;
      video.pause();
      video.removeAttribute("src");
      video.load();
    };
  }, [src, autoplay]);

  return (
    <div className="player-wrap">
      <video
        ref={videoRef}
        className="player-video"
        controls
        autoPlay={autoplay}
        playsInline
      />
      {error && <p className="err">{error}</p>}
    </div>
  );
}
