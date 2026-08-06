import QrPanel from "../components/QrPanel";
import type { Channel, ServerInfo } from "../services/types";

// "Open on TV" page: shows the LAN URL of the embedded server as a QR
// code. The TV browser opens it (Samsung Internet etc.) and gets the
// full channel browsing + playback UI served by the Rust backend.
export default function TvCast({
  serverInfo,
  channel,
}: {
  serverInfo: ServerInfo | null;
  channel: Channel | null;
}) {
  if (!serverInfo) {
    return <main className="page"><p className="muted">TV server starting…</p></main>;
  }

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
        <ul className="ip-list">
          {serverInfo.ips.map((ip) => {
            const url = `http://${ip}:${serverInfo.port}/#/channels`;
            return (
              <li key={ip}>
                <code>{url}</code>
                <button onClick={() => copy(url)}>Copy</button>
              </li>
            );
          })}
        </ul>
        <p className="muted small">
          Your phone/TV must be on the same Wi-Fi network. Allow inbound connections when the
          firewall asks (Windows: Private networks).
        </p>
      </div>
    </main>
  );
}
