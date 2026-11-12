-- Frozen Ledger schema version 4, appended to v1 + v2 + v3.

CREATE TABLE ledger_order (
    seq     INTEGER PRIMARY KEY AUTOINCREMENT,
    fact_id INTEGER UNIQUE REFERENCES fact (id)
) STRICT;

INSERT INTO ledger_order (fact_id) SELECT id FROM fact ORDER BY recorded_at, id;

CREATE TRIGGER fact_takes_its_place_in_order AFTER INSERT ON fact
BEGIN INSERT INTO ledger_order (fact_id) VALUES (new.id); END;

CREATE INDEX fact_by_successor ON fact (superseded_by);

CREATE TABLE ledger (
    id          INTEGER PRIMARY KEY REFERENCES ledger_order (seq),
    run_id      INTEGER NOT NULL REFERENCES run (id),
    at          INTEGER NOT NULL,
    store       TEXT NOT NULL CHECK (store IN ('library', 'sidecar')),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('dj', 'agent', 'tool', 'web')),
    source_name TEXT NOT NULL,
    action      TEXT NOT NULL CHECK (action IN ('edit', 'undo', 'redo', 'write')),
    kind        TEXT NOT NULL,
    label       TEXT NOT NULL,
    command     TEXT,
    reverses    INTEGER,
    part_of     INTEGER,
    reversible  INTEGER NOT NULL CHECK (reversible IN (0, 1)),
    format      INTEGER NOT NULL,
    payload     TEXT
) STRICT;

CREATE INDEX ledger_by_run ON ledger (run_id);
CREATE INDEX ledger_by_reversed ON ledger (reverses);
CREATE INDEX ledger_by_whole ON ledger (part_of);

CREATE TABLE ledger_intent (
    id          INTEGER PRIMARY KEY REFERENCES ledger_order (seq),
    run_id      INTEGER REFERENCES run (id),
    at          INTEGER NOT NULL,
    source_kind TEXT NOT NULL,
    source_name TEXT NOT NULL,
    description TEXT NOT NULL,
    command     TEXT
) STRICT;

CREATE TABLE ledger_owned_run (
    run_id INTEGER PRIMARY KEY REFERENCES run (id)
) STRICT;

CREATE TRIGGER ledger_is_never_changed BEFORE UPDATE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never changed'); END;

CREATE TRIGGER ledger_is_never_removed BEFORE DELETE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never removed'); END;

UPDATE meta SET value = '4' WHERE key = 'schema_version';
