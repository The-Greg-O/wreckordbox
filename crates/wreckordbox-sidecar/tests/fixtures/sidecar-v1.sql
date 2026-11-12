-- A Sidecar store exactly as schema version 1 made it (foundation before
-- wbx-35), with one Run and two Facts. Frozen: never edit this file to follow
-- the current schema; a migration test opens it as an old store.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE kind (
    name   TEXT PRIMARY KEY,
    nature TEXT NOT NULL CHECK (nature IN ('judgement', 'measurement'))
) STRICT;

CREATE TABLE run (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    description TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    outcome     TEXT
) STRICT;

CREATE TABLE fact (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id      TEXT NOT NULL,
    fingerprint   TEXT,
    kind          TEXT NOT NULL REFERENCES kind (name),
    value         TEXT NOT NULL,
    source_kind   TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name   TEXT NOT NULL,
    confidence    REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    run_id        INTEGER NOT NULL REFERENCES run (id),
    recorded_at   INTEGER NOT NULL,
    stale_at      INTEGER,
    superseded_at INTEGER,
    superseded_by INTEGER REFERENCES fact (id)
) STRICT;

CREATE INDEX fact_by_track ON fact (track_id, kind);
CREATE INDEX fact_by_fingerprint ON fact (fingerprint, kind);
CREATE INDEX fact_by_run ON fact (run_id);

INSERT INTO meta (key, value) VALUES ('schema_version', '1');
INSERT INTO kind (name, nature) VALUES ('genre', 'judgement'), ('intensity', 'measurement');
INSERT INTO run (id, source_kind, source_name, description, started_at, finished_at, outcome)
VALUES (7, 'agent', 'tagger', 'Tag the new arrivals', 1794400000000, 1794400001000, 'recorded 2 Facts');
INSERT INTO fact (id, track_id, fingerprint, kind, value, source_kind, source_name, confidence,
                  run_id, recorded_at, stale_at, superseded_at, superseded_by)
VALUES (3, '10003', NULL, 'genre', '"Techno"', 'agent', 'tagger', 0.7, 7, 1794400000100, NULL, 1794400000500, 4),
       (4, '10003', 'a1:00ff', 'genre', '"Tech House"', 'agent', 'tagger', 0.8, 7, 1794400000500, NULL, NULL, NULL);
