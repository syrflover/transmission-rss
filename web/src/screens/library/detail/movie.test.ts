import assert from "node:assert/strict";
import { test } from "node:test";

import { MOVIE_EPISODE, MOVIE_LABEL, candidateKey, movieKey, rowLabel, targetOf, unitOf } from "./movie.ts";

/** A candidate as the attachment reads it: the key of its episode text, and the movie the server names for it. */
const candidate = (episode_key: string, movie_episode?: number) => ({ episode_key, movie_episode });

/** The `episode_key` of the movie-only work's one library episode, `1`. */
const MOVIE_ROW = "n:1";

test("the one row of a movie-only work is 본편, every other row is its episode", () => {
  assert.equal(MOVIE_LABEL, "본편");
  assert.equal(rowLabel("1", true), "본편");
  assert.equal(rowLabel("1", false), "1화");
  assert.equal(rowLabel("12", false), "12화");
  assert.equal(rowLabel("SP", false), "SP");
});

test("a movie's key is the key of its library episode", () => {
  assert.equal(movieKey(MOVIE_EPISODE), MOVIE_ROW);
  assert.equal(movieKey(2), "n:2");
});

test("a movie's candidates attach to its row whatever their episode text says", () => {
  // Anissia writes a movie's episode as `0`, `1`, `극장판` or leaves it blank.
  for (const key of ["n:0", "n:1", "n:7", "t:극장판", "t:SP", "t:"]) {
    assert.equal(candidateKey(candidate(key, MOVIE_EPISODE)), MOVIE_ROW, key);
  }
});

test("a season's candidates attach to the episode their text names", () => {
  assert.equal(candidateKey(candidate("n:13")), "n:13");
  assert.equal(candidateKey(candidate("n:0")), "n:0");
  assert.equal(candidateKey(candidate("t:SP")), "t:SP");
  assert.equal(candidateKey({ episode_key: "n:3", movie_episode: undefined }), "n:3");
  assert.equal(candidateKey({ episode_key: "n:3", movie_episode: null }), "n:3");
});

test("a link names its movie, or its season", () => {
  assert.deepEqual(targetOf({ season: 0, movie: 1 }), { kind: "movie", episode: 1 });
  assert.deepEqual(targetOf({ season: 2 }), { kind: "season", season: 2 });
  assert.equal(unitOf({ movie: 1 }), "극장판");
  assert.equal(unitOf({}), "시즌");
});
