-- Frozen version 3 (Decisions): append to v1 + v2. Do not follow later schema changes.

CREATE TABLE decision (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    question_type TEXT NOT NULL CHECK (question_type IN ('choice', 'yes_no')),
    topic         TEXT,
    question      TEXT NOT NULL,
    context       TEXT NOT NULL,
    candidates    TEXT NOT NULL,
    track_id      TEXT,
    run_id        INTEGER REFERENCES run (id),
    created_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE decision_answer (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    decision_id        INTEGER NOT NULL REFERENCES decision (id),
    decider_kind       TEXT NOT NULL CHECK (decider_kind IN ('dj', 'agent', 'tool', 'web')),
    decider_name       TEXT NOT NULL,
    choice             TEXT NOT NULL,
    choice_probability REAL NOT NULL CHECK (choice_probability >= 0 AND choice_probability <= 1),
    probabilities      TEXT,
    reason             TEXT,
    fact_id            INTEGER REFERENCES fact (id),
    run_id             INTEGER REFERENCES run (id),
    answered_at        INTEGER NOT NULL
) STRICT;

-- Append-only, held by the store itself: a Decision and an answer are
-- written once.
-- INSERT OR REPLACE deletes the row it replaces without firing a DELETE
-- trigger (unless the connection has recursive triggers on), so taking an
-- identifier that is in use is refused here.
CREATE TRIGGER decision_is_never_replaced BEFORE INSERT ON decision
WHEN EXISTS (SELECT 1 FROM decision WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'a Decision is never replaced'); END;
CREATE TRIGGER decision_answer_is_never_replaced BEFORE INSERT ON decision_answer
WHEN EXISTS (SELECT 1 FROM decision_answer WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'an answer to a Decision is never replaced'); END;
CREATE TRIGGER decision_is_never_changed BEFORE UPDATE ON decision
BEGIN SELECT RAISE(ABORT, 'a Decision is never changed'); END;
CREATE TRIGGER decision_is_never_deleted BEFORE DELETE ON decision
BEGIN SELECT RAISE(ABORT, 'a Decision is never deleted'); END;
CREATE TRIGGER decision_answer_is_never_changed BEFORE UPDATE ON decision_answer
BEGIN SELECT RAISE(ABORT, 'an answer to a Decision is never changed'); END;
CREATE TRIGGER decision_answer_is_never_deleted BEFORE DELETE ON decision_answer
BEGIN SELECT RAISE(ABORT, 'an answer to a Decision is never deleted'); END;

CREATE INDEX decision_by_track ON decision (track_id);
CREATE INDEX decision_by_topic ON decision (topic);
CREATE INDEX decision_answer_by_decision ON decision_answer (decision_id);
CREATE INDEX decision_answer_by_decider ON decision_answer (decider_kind, decider_name);

INSERT INTO decision (id,question_type,question,context,candidates,track_id,run_id,created_at)
VALUES (2,'choice','Genre?','{}','[{"name":"Techno"},{"name":"House"}]','10003',7,1794400002000);
UPDATE meta SET value = '3' WHERE key = 'schema_version';
