import { useCallback, useEffect, useState } from "react";
import Home from "./pages/Home";
import Channels from "./pages/Channels";
import TvCast from "./pages/TvCast";
import PlayerView from "./pages/PlayerView";
import { api } from "./services/api";
import type { Channel, ServerInfo } from "./services/types";

type Tab = "home" | "channels" | "cast";

// Minimal app-level state; shared via props to keep the MVP simple.
export default function App() {
  const [tab, setTab] = useState<Tab>("home");
  const [serverInfo, setServerInfo] = useState<ServerInfo | null>(null);
  const [activeChannel, setActiveChannel] = useState<Channel | null>(null);
  const [showPlayer, setShowPlayer] = useState(false);

  const refreshServerInfo = useCallback(() => {
    api
      .getServerInfo()
      .then(setServerInfo)
      .catch((e) => console.error("get_server_info failed:", e));
  }, []);

  useEffect(() => {
    refreshServerInfo();
    const t = setInterval(refreshServerInfo, 5000);
    return () => clearInterval(t);
  }, [refreshServerInfo]);

  const playChannel = (ch: Channel) => {
    setActiveChannel(ch);
    setShowPlayer(true);
  };

  return (
    <div className="app">
      <header className="topbar">
        <h1>📺 AndroIPTV</h1>
        <nav className="tabs">
          <button className={tab === "home" ? "tab active" : "tab"} onClick={() => setTab("home")}>
            Playlists
          </button>
          <button className={tab === "channels" ? "tab active" : "tab"} onClick={() => setTab("channels")}>
            Channels
          </button>
          <button className={tab === "cast" ? "tab active" : "tab"} onClick={() => setTab("cast")}>
            TV Server
          </button>
        </nav>
        {serverInfo && (
          <span className="server-chip" title="Local TV server">
            {serverInfo.url}
          </span>
        )}
      </header>

      {tab === "home" && <Home onPlaylistImported={refreshServerInfo} />}
      {tab === "channels" && <Channels onPlayChannel={playChannel} />}
      {tab === "cast" && <TvCast serverInfo={serverInfo} channel={activeChannel} />}

      {showPlayer && activeChannel && (
        <PlayerView
          channel={activeChannel}
          serverInfo={serverInfo}
          onClose={() => setShowPlayer(false)}
        />
      )}
    </div>
  );
}
