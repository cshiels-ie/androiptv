import { useMemo, useRef, useState } from "react";
import type { Channel } from "../services/types";

// Simple windowing: renders a slice of the list, grows it as the user
// scrolls. Keeps 100k-channel groups usable without a dependency.
export default function ChannelList({
  channels,
  onPlay,
  emptyLabel = "No channels",
}: {
  channels: Channel[];
  onPlay: (ch: Channel) => void;
  emptyLabel?: string;
}) {
  const [visible, setVisible] = useState(300);
  const scrollRef = useRef<HTMLDivElement>(null);
  const slice = useMemo(() => channels.slice(0, visible), [channels, visible]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 400) {
      setVisible((v) => Math.min(v + 300, channels.length));
    }
  };

  if (channels.length === 0) return <p className="muted">{emptyLabel}</p>;

  return (
    <div className="channel-list" ref={scrollRef} onScroll={onScroll}>
      {slice.map((ch) => (
        <button key={ch.id} className="channel-item" onClick={() => onPlay(ch)}>
          {ch.logo_url ? (
            <img className="ch-logo" src={ch.logo_url} alt="" loading="lazy" />
          ) : (
            <span className="ch-logo placeholder">📺</span>
          )}
          <span className="ch-name">{ch.name}</span>
          {ch.tvg_chno != null && <span className="ch-chno">{ch.tvg_chno}</span>}
        </button>
      ))}
      {visible < channels.length && <p className="muted center">scroll for more…</p>}
    </div>
  );
}
