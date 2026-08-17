import { useMemo, useRef, useState } from "react";
import { logoSrc } from "../services/api";
import type { Channel } from "../services/types";
import { TvIcon } from "./icons";

// Simple windowing: renders a slice of the list, grows it as the user
// scrolls. Keeps 100k-channel groups usable without a dependency.
export default function ChannelList({
  channels,
  onPlay,
  emptyLabel = "No channels",
  serverUrl,
}: {
  channels: Channel[];
  onPlay: (ch: Channel) => void;
  emptyLabel?: string;
  serverUrl: string | null;
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
      {slice.map((ch) => {
        const logo = logoSrc(ch.logo_url, serverUrl);
        return (
          <button key={ch.id} className="channel-item" onClick={() => onPlay(ch)}>
            {logo ? (
              <img
                className="ch-logo"
                src={logo}
                alt=""
                loading="lazy"
                onError={(e) => (e.currentTarget.style.display = "none")}
              />
            ) : (
              <span className="ch-logo placeholder">
                <TvIcon />
              </span>
            )}
            <span className="ch-name">{ch.name}</span>
            {ch.tvg_chno != null && <span className="ch-chno">{ch.tvg_chno}</span>}
          </button>
        );
      })}
      {visible < channels.length && <p className="muted center">scroll for more…</p>}
    </div>
  );
}
