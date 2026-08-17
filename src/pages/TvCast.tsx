import { useEffect, useState } from "react";
import QrPanel from "../components/QrPanel";
import { api } from "../services/api";
import type { Channel, ServerInfo } from "../services/types";

// "Open on TV" page: shows the LAN URL of the embedded server as a QR
// code. The TV browser opens it (Samsung Internet etc.) and gets the
// full channel browsing + playback UI served by the Rust backend.
export default function TvCast({
  serverInfo,
  channel,
  onServerInfo,
}: {
  serverInfo: ServerInfo | null;
  channel: Channel | null;
  onServerInfo?: (info: ServerInfo) => void;
}) {
  if (!serverInfo) {
    return <main className="page"><p className="muted">TV server starting…</p></main>;
  }
  return <TvCastPanel serverInfo={serverInfo} channel={channel} onServerInfo={onServerInfo} />;
}

function TvCastPanel({
  serverInfo,
  channel,
  onServerInfo,
}: {
  serverInfo: ServerInfo;
  channel: Channel | null;
  onServerInfo?: (info: ServerInfo) => void;
}) {
  // "__auto__" (automatic detection) | "__custom__" (free text) | a detected IP.
  const [hostChoice, setHostChoice] = useState("__auto__");
  const [customHost, setCustomHost] = useState("");
  const [portInput, setPortInput] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  // Per-address reachability probe, true/false per detected IP. The app
  // fetches each http://<ip>:<port>/api/status through the same network
  // path a TV would use, so the UI can show which address actually works
  // instead of silently advertising a dead one (wrong interface, VPN,
  // router isolation, cleartext policy …).
  const [reach, setReach] = useState<Record<string, boolean>>({});

  // Reflect the server's stored prefs into the form (initial load,
  // restart, or change from another source).
  useEffect(() => {
    const o = serverInfo.ip_override;
    setHostChoice(o ? (serverInfo.ips.includes(o) ? o : "__custom__") : "__auto__");
    setCustomHost(o ?? "");
  }, [serverInfo.ip_override, serverInfo.ips]);
  useEffect(() => {
    setPortInput(serverInfo.port_pref != null ? String(serverInfo.port_pref) : "");
  }, [serverInfo.port_pref]);

  useEffect(() => {
    let alive = true;
    setReach({});
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 4000);
    for (const ip of serverInfo.ips) {
      fetch(`http://${ip}:${serverInfo.port}/api/status`, {
        signal: controller.signal,
        cache: "no-store",
      })
        .then((r) => r.ok)
        .catch(() => false)
        .then((ok) => {
          if (alive) setReach((prev) => ({ ...prev, [ip]: ok }));
        });
    }
    return () => {
      alive = false;
      clearTimeout(timer);
      controller.abort();
    };
  }, [serverInfo.ips, serverInfo.port]);

  const save = (host: string, port: string) => {
    setSaveError(null);
    const ip = host === "__auto__" ? null : host;
    let p: number | null = null;
    if (port.trim() !== "") {
      p = Number(port);
      if (!Number.isInteger(p) || p < 1 || p > 65535) {
        setSaveError("Port must be a whole number between 1 and 65535.");
        return;
      }
    }
    api
      .setServerPrefs(ip, p)
      .then(onServerInfo)
      .catch((e) => setSaveError(String(e)));
  };

  const browseUrl = `${serverInfo.url}/#/channels`;
  const channelUrl = channel ? `${serverInfo.url}/#/play/${channel.id}` : null;

  const copy = (text: string) => {
    navigator.clipboard?.writeText(text).catch(() => {});
  };

  return (
    <main className="page cast-page">
      <div className="panel">
        <h2>Open on TV</h2>
        <p className="muted">
          On your Smart TV, open the browser (e.g. Samsung Internet) and go to the URL below —
          or scan the QR code with a phone and share the link to the TV. No casting app needed:
          this device streams channels directly to the TV over your local network.
        </p>

        <div className="cast-grid">
          <QrPanel url={browseUrl} label="Browse channels" />
          {channel && channelUrl && <QrPanel url={channelUrl} label={`Play: ${channel.name}`} />}
        </div>

        <h3>Server address</h3>
        <div className="host-form">
          <label>
            Host
            <select
              value={hostChoice}
              onChange={(e) => {
                const v = e.target.value;
                setHostChoice(v);
                if (v !== "__custom__") save(v, portInput);
              }}
              title="Which address the server advertises (QR codes, status bar, playback URLs)"
            >
              <option value="__auto__">Automatic — best detected IP</option>
              {serverInfo.ips.map((ip) => (
                <option key={ip} value={ip}>
                  {ip}
                </option>
              ))}
              <option value="__custom__">Custom hostname or IP…</option>
            </select>
          </label>
          {hostChoice === "__custom__" && (
            <label>
              Custom host
              <input
                type="text"
                value={customHost}
                placeholder="e.g. 192.168.1.50 or my-tv-server.local"
                onChange={(e) => setCustomHost(e.target.value)}
                onBlur={() => {
                  const h = customHost.trim();
                  if (h === "") {
                    setHostChoice("__auto__");
                    save("__auto__", portInput);
                  } else {
                    save(h, portInput);
                  }
                }}
                onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
              />
            </label>
          )}
          <label>
            Port
            <input
              type="number"
              min={1}
              max={65535}
              value={portInput}
              placeholder="4040"
              onChange={(e) => setPortInput(e.target.value)}
              onBlur={() => save(hostChoice, portInput)}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
            />
            <span className="muted small">
              {serverInfo.port_pref
                ? "applies after restart (next start uses it)"
                : "applies after restart — currently on " + serverInfo.port}
            </span>
          </label>
        </div>
        {saveError && <p className="err">{saveError}</p>}

        <h3>Detected addresses</h3>
        <ul className="ip-list">
          {serverInfo.ips.map((ip) => {
            const url = `http://${ip}:${serverInfo.port}/#/channels`;
            const state = reach[ip];
            return (
              <li key={ip}>
                <code>{url}</code>
                {state === undefined && <span className="muted small">checking…</span>}
                {state === false && (
                  <span className="err small">no response from this device</span>
                )}
                {state === true && <span className="ok">reachable</span>}
                <button onClick={() => copy(url)}>Copy</button>
              </li>
            );
          })}
          {serverInfo.ips.length === 0 && (
            <li className="muted">No LAN addresses detected — check your network connection.</li>
          )}
        </ul>
        {serverInfo.ips.length > 0 &&
          Object.keys(reach).some((ip) => reach[ip] === true) &&
          reach[serverInfo.host] === false && (
            <p className="err">
              The advertised address ({serverInfo.host}) doesn&apos;t respond from this device.
              Pick a reachable one from the Host selector above, then scan its QR again.
            </p>
          )}
        {serverInfo.ips.length > 0 &&
          Object.keys(reach).length === serverInfo.ips.length &&
          Object.values(reach).every((ok) => ok === false) && (
            <p className="err">
              None of the detected addresses respond from this device. The server may not be
              running, your router may block device-to-device traffic (AP/client isolation), or
              this app build may not allow cleartext http (CI builds do).
            </p>
          )}
        <p className="muted small">
          Your phone/TV must be on the same Wi-Fi network. Allow inbound connections when the
          firewall asks (Windows: Private networks), and if your router has AP/client isolation
          enabled, turn it off — it blocks devices from talking to each other. If the
          auto-detected address isn&apos;t the one your TV can reach, pick a different IP above or
          type your router&apos;s address for this device.
        </p>
      </div>
    </main>
  );
}
