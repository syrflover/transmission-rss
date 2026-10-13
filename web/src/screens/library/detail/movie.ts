import { episodeLabel } from "./episodeKey.ts";

/**
 * What the work detail and the to-do screens know of a movie (극장판): an episode of season 0 that has an Anissia link
 * of its own (`docs/adr/0018-movies-in-season-zero-with-their-own-link.md`). A movie-only work (`movie_only`) is one
 * such episode and nothing else; its row is called `본편`. It imports only `episodeKey.ts`, not even the API's types
 * (whose module has `@/` imports), so it runs and is type-checked under `node --test`.
 */

/** What a movie-only work's one episode is called, in place of `1화`. */
export const MOVIE_LABEL = "본편";

/** The number of the movie of a movie-only work: season 0's episode 1. */
export const MOVIE_EPISODE = 1;

/** Where an Anissia link, its search and its candidates are: a season, or a movie (an episode of season 0). */
export type AnissiaTarget = { kind: "season"; season: number } | { kind: "movie"; episode: number };

/** The target of a link: a movie's link carries `movie`, its episode number. */
export function targetOf(link: { season: number; movie?: number }): AnissiaTarget {
  return link.movie === undefined ? { kind: "season", season: link.season } : { kind: "movie", episode: link.movie };
}

/** The word the screen uses for what the link is of. */
export function unitOf(link: { movie?: number }): "극장판" | "시즌" {
  return link.movie === undefined ? "시즌" : "극장판";
}

/** The `episode_key` of a movie's episode, in the form the server gives a whole number (`n:1`). */
export function movieKey(episode: number): string {
  return `n:${episode}`;
}

/**
 * The episode a candidate is about. A movie's candidates (`movie_episode`) are all about the movie, whatever their
 * episode text says (`0`, `1`, `극장판`, `SP`); any other candidate is about the episode its text names.
 */
export function candidateKey(c: { episode_key: string; movie_episode?: number | null }): string {
  return c.movie_episode == null ? c.episode_key : movieKey(c.movie_episode);
}

/** What an episode's row is called: `본편` for the one episode of a movie-only work, else `12화` (`SP`). */
export function rowLabel(shown: string, movieOnly: boolean): string {
  return movieOnly ? MOVIE_LABEL : episodeLabel(shown);
}
