-- Commands the web accepts and the worker carries out (see `store::commands`).
--
-- `id` is the command ID the browser made for one user action; it makes a
-- repeated delivery of the same action harmless. `payload` is the request's
-- content in a canonical JSON form, kept to tell a repeat from a different
-- request that reuses the ID. `subject` names what the command is about, so
-- the screen can find the commands of a list row without opening every
-- payload. `state` is `pending` (accepted, not started), `running` (a worker
-- started it and has not finished), `done` or `failed`. `outcome` is a JSON
-- object written when the command ends and never holds secret values.
-- `seq` is the order of acceptance. Timestamps are Unix milliseconds.

CREATE TABLE commands (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    id          TEXT    NOT NULL UNIQUE CHECK (id <> ''),
    kind        TEXT    NOT NULL CHECK (kind <> ''),
    payload     TEXT    NOT NULL,
    subject     TEXT,
    state       TEXT    NOT NULL CHECK (state IN ('pending', 'running', 'done', 'failed')),
    attempts    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    finished_at INTEGER,
    outcome     TEXT
);

-- What the worker polls: the open commands, in the order they were accepted.
CREATE INDEX commands_open ON commands (seq) WHERE state IN ('pending', 'running');
CREATE INDEX commands_by_subject ON commands (kind, subject, seq DESC);
