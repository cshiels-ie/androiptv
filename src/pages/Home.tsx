import PlaylistManager from "../components/PlaylistManager";

export default function Home({ onPlaylistImported }: { onPlaylistImported: () => void }) {
  return (
    <main className="page">
      <PlaylistManager onImported={onPlaylistImported} />
    </main>
  );
}
