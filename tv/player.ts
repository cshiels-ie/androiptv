import Hls from "hls.js";

// Attach a playback URL to a <video>. Prefers native HLS (some TV
// browsers — Samsung Internet on newer Tizen — decode it natively),
// falls back to hls.js when MSE is available. Returns a cleanup fn.
export function attachPlayer(
  video: HTMLVideoElement,
  src: string,
  onError: (msg: string) => void,
): () => void {
  let hls: Hls | null = null;

  if (Hls.isSupported()) {
    hls = new Hls({ liveDurationInfinity: true, enableWorker: false });
    hls.loadSource(src);
    hls.attachMedia(video);
    hls.on(Hls.Events.ERROR, (_e, data) => {
      if (data.fatal) {
        onError("Stream error — channel may be offline.");
        hls?.destroy();
      }
    });
    // Wait for the manifest before play(): on weak TV engines calling
    // play() right after attachMedia rejects (nothing buffered yet).
    hls.on(Hls.Events.MANIFEST_PARSED, () => {
      video.play().catch(() => {});
    });
  } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
    video.src = src;
    video.play().catch(() => {});
  } else {
    onError("This TV browser cannot play HLS streams.");
  }

  return () => {
    hls?.destroy();
    hls = null;
    video.pause();
    video.removeAttribute("src");
    video.load();
  };
}
