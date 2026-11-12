-- What schema version 2 (Mutants, wbx-78) added to a store, with one Mutant
-- and its first change. Run on top of sidecar-v1.sql it is a store exactly
-- as version 2 made it, before Decisions. Frozen: never edit this file to
-- follow the current schema; a migration test opens it as an old store.
CREATE TABLE mutant (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id TEXT NOT NULL,
    entry_id    TEXT NOT NULL,
    track_id    TEXT NOT NULL,
    fingerprint TEXT,
    created_at  INTEGER NOT NULL,
    created_run INTEGER NOT NULL REFERENCES run (id),
    removed_at  INTEGER,
    removed_run INTEGER REFERENCES run (id)
) STRICT;

CREATE UNIQUE INDEX mutant_live_entry ON mutant (entry_id) WHERE removed_at IS NULL;
CREATE INDEX mutant_by_playlist ON mutant (playlist_id);
CREATE INDEX mutant_by_track ON mutant (track_id);

CREATE TABLE mutant_change (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    mutant_id   INTEGER NOT NULL REFERENCES mutant (id),
    action      TEXT NOT NULL,
    edits       TEXT NOT NULL,
    fingerprint TEXT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    run_id      INTEGER NOT NULL REFERENCES run (id),
    recorded_at INTEGER NOT NULL
) STRICT;

CREATE INDEX mutant_change_by_mutant ON mutant_change (mutant_id);
CREATE INDEX mutant_change_by_run ON mutant_change (run_id);

INSERT INTO run (id, source_kind, source_name, description, started_at, finished_at, outcome)
VALUES (8, 'dj', 'dj', 'Mutate an entry', 1794400002000, 1794400002000, 'created Mutant 2');
INSERT INTO mutant (id, playlist_id, entry_id, track_id, fingerprint, created_at, created_run,
                    removed_at, removed_run)
VALUES (2, '40', '400', '10003', 'a1:00ff', 1794400002000, 8, NULL, NULL);
INSERT INTO mutant_change (id, mutant_id, action, edits, fingerprint, source_kind, source_name,
                           run_id, recorded_at)
VALUES (5, 2, 'created', '{"gain_db":-3}', 'a1:00ff', 'dj', 'dj', 8, 1794400002000);
UPDATE meta SET value = '2' WHERE key = 'schema_version';
