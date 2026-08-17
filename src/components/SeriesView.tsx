import { useEffect, useMemo, useState } from "react";
import { api, logoSrc } from "../services/api";
import type { Channel, Episode } from "../services/types";
import { FilmIcon } from "./icons";

export default function SeriesView({
  series,
  onBack,
  onPlayEpisode,
  serverUrl,
}: {
  series: Channel;
  onBack: () => void;
  onPlayEpisode: (series: Channel, ep: Episode) => void;
  serverUrl: string | null;
}) {
  const [episodes, setEpisodes] = useState<Episode[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [openSeasons, setOpenSeasons] = useState<Set<number>>(new Set());

  useEffect(() => {
    api
      .seriesEpisodes(series.id)
      .then(setEpisodes)
      .catch((e) => setError(String(e)));
  }, [series.id]);

  // Group by season, each sorted by episode number, seasons sorted numerically.
  const bySeason = useMemo(() => {
    const map = new Map<number, Episode[]>();
    for (const ep of episodes ?? []) {
      const list = map.get(ep.season);
      if (list) list.push(ep);
      else map.set(ep.season, [ep]);
    }
    return [...map.entries()]
      .map(([season, eps]) => ({
        season,
        episodes: [...eps].sort((a, b) => a.episode_num - b.episode_num),
      }))
      .sort((a, b) => a.season - b.season);
  }, [episodes]);

  // Auto-open the first season once episodes load, unless one is open already.
  useEffect(() => {
    if (episodes === null || episodes.length === 0 || openSeasons.size > 0)
      return;
    const first = bySeason[0]?.season;
    if (first !== undefined) setOpenSeasons(new Set([first]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [episodes]);

  const toggleSeason = (season: number) => {
    setOpenSeasons((prev) => {
      const next = new Set(prev);
      if (next.has(season)) next.delete(season);
      else next.add(season);
      return next;
    });
  };

  return (
    <div className="series-view">
      <div className="series-header">
        <button className="back-link" onClick={onBack}>
          ← Back to series
        </button>
        {series.logo_url && (
          <img
            className="series-poster"
            src={logoSrc(series.logo_url, serverUrl) ?? series.logo_url}
            alt=""
            onError={(e) => (e.currentTarget.style.display = "none")}
          />
        )}
        <h2 className="series-title">{series.name}</h2>
      </div>
      {episodes === null && !error && <p className="muted">Loading episodes…</p>}
      {error && <p className="err">{error}</p>}
      {episodes !== null && episodes.length === 0 && (
        <p className="muted">No episodes.</p>
      )}
      {episodes !== null && episodes.length > 0 && (
        <>
          {bySeason.map(({ season, episodes: eps }) => (
            <div className="season-block" key={season}>
              <button
                className="season-title"
                onClick={() => toggleSeason(season)}
              >
                Season {season} {openSeasons.has(season) ? "▾" : "▸"}
              </button>
              {openSeasons.has(season) && (
                <div className="season-body">
                  {eps.map((ep) => (
                    <button
                      key={ep.id}
                      className="channel-item"
                      onClick={() => onPlayEpisode(series, ep)}
                    >
                      {ep.logo_url ? (
                        <img
                          className="ch-logo"
                          src={logoSrc(ep.logo_url, serverUrl) ?? ep.logo_url}
                          alt=""
                          loading="lazy"
                          onError={(e) => (e.currentTarget.style.display = "none")}
                        />
                      ) : (
                        <span className="ch-logo placeholder">
                          <FilmIcon />
                        </span>
                      )}
                      <span className="ch-name">
                        {ep.episode_num}. {ep.title}
                      </span>
                      <span className="ch-chno">▶</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ))}
        </>
      )}
    </div>
  );
}
