import { useEffect, useState } from "react";
import QRCode from "qrcode";

export default function QrPanel({ url, label }: { url: string; label: string }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    QRCode.toDataURL(url, { width: 260, margin: 1, color: { dark: "#0b0f14" } })
      .then((d) => alive && setDataUrl(d))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [url]);

  return (
    <div className="qr-panel" title={label}>
      {dataUrl ? (
        <img src={dataUrl} alt={label} />
      ) : (
        <div className="qr-placeholder">…</div>
      )}
      <p className="qr-label">{label}</p>
    </div>
  );
}
