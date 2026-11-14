BEGIN TRANSACTION;
CREATE TABLE attachment_change (
    id INTEGER PRIMARY KEY,
    from_track TEXT NOT NULL,
    to_track TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    attached_at INTEGER NOT NULL,
    run_id INTEGER REFERENCES run(id)
) STRICT;
CREATE TABLE attachment_review (
    track_id TEXT NOT NULL,
    from_track TEXT NOT NULL,
    reason TEXT NOT NULL,
    PRIMARY KEY(track_id, from_track)
) STRICT;
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
CREATE TABLE dj_fact (fact_id INTEGER PRIMARY KEY REFERENCES fact(id), ledger_id INTEGER NOT NULL REFERENCES ledger(id), reconciled INTEGER NOT NULL);
CREATE TABLE dj_fact_effect (
 entry_id INTEGER NOT NULL REFERENCES ledger(id), fact_id INTEGER NOT NULL REFERENCES fact(id),
 existed INTEGER NOT NULL, superseded_at INTEGER, superseded_by INTEGER,
 PRIMARY KEY(entry_id,fact_id)
);
CREATE TABLE dj_intent (id INTEGER PRIMARY KEY REFERENCES ledger_intent(id) ON DELETE CASCADE, snapshot TEXT NOT NULL);
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
INSERT INTO "fact" VALUES(1,'mp3-xing','a1:6f6f98dffb797172f3b4e5497823b82980b251b41a351763d4dc47afecb8c878','stems','{"produced_by":{"engine_version":"synthetic","fixture":"foundation ec1d477d"},"sample_rate":44100,"samples":89856,"stems":[{"bits_per_sample":16,"channels":1,"file":"stems/mp3-xing/run-1/vocals.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"vocals"},{"bits_per_sample":16,"channels":1,"file":"stems/mp3-xing/run-1/other.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"other"},{"bits_per_sample":16,"channels":1,"file":"stems/mp3-xing/run-1/bass.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"bass"},{"bits_per_sample":16,"channels":1,"file":"stems/mp3-xing/run-1/drums.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"drums"}]}','tool','separation-engine',1.0,1,1794656491164,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(2,'mp3-xing','a1:6f6f98dffb797172f3b4e5497823b82980b251b41a351763d4dc47afecb8c878','intensity','{"audio":{"sample_rate":44100,"seconds":2.038},"bars":null,"cells":{"flux":[1.2383,1.7248,1.4472,1.5752,1.6147,1.434,1.6352,1.3803,1.5718,1.643,0.1541],"high_end":[-40.2,-38.63,-38.56,-38.45,-38.68,-38.75,-38.28,-38.76,-38.6,-38.32,-39.23],"loudness":[-26.79,-26.33,-26.16,-26.43,-26.46,-26.44,-26.05,-26.55,-26.24,-26.23,-27.57],"low_end":[-34.55,-33.69,-33.84,-33.66,-34.55,-34.64,-32.75,-34.3,-32.33,-32.03,-31.06],"onsets":[0,0,0,0,0,0,0,0,0,0,0],"step_seconds":0.2},"curve":{"features":{"high_end":[-38.86,-38.54,-39.23],"loudness":[-26.43,-26.3,-27.57],"low_end":[-34.04,-33.08,-31.06],"onset_density":[0.0,0.0,0.0],"onset_strength":[7.6002,7.6643,4.0553]},"intensity":[0.3818,0.3886,0.3258],"offset_seconds":0.0,"step_seconds":1.0},"grid":null,"method":{"features":["loudness","low_end","high_end","onset_strength","onset_density"],"name":"wreckordbox-intensity","scales":{"high_end":{"ceiling":-20.0,"floor":-60.0},"loudness":{"ceiling":-5.0,"floor":-40.0},"low_end":{"ceiling":-10.0,"floor":-50.0},"onset_density":{"ceiling":8.0,"floor":0.0},"onset_strength":{"ceiling":10.0,"floor":0.0}},"version":2,"weights":{"high_end":0.1,"loudness":0.4,"low_end":0.15,"onset_density":0.2,"onset_strength":0.15}},"summary":{"features":{"high_end":-38.7,"loudness":-26.38,"low_end":-33.47,"onset_density":0.0,"onset_strength":7.5656},"high":0.3886,"intensity":0.3654,"loudness_integrated_lufs":-26.35,"loudness_short_term_max_lufs":-26.41,"low":0.3258}}','tool','wreckordbox-intensity',1.0,2,1794656491192,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(3,'mp3-cbr','a1:6f6f98dffb797172f3b4e5497823b82980b251b41a351763d4dc47afecb8c878','stems','{"produced_by":{"engine_version":"synthetic","fixture":"foundation ec1d477d"},"sample_rate":44100,"samples":89856,"stems":[{"bits_per_sample":16,"channels":1,"file":"stems/mp3-cbr/run-3/vocals.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"vocals"},{"bits_per_sample":16,"channels":1,"file":"stems/mp3-cbr/run-3/other.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"other"},{"bits_per_sample":16,"channels":1,"file":"stems/mp3-cbr/run-3/bass.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"bass"},{"bits_per_sample":16,"channels":1,"file":"stems/mp3-cbr/run-3/drums.wav","format":"wav","hash":"b3:6aad5e80a9bba21b95c9e94f098b01ce75e79153c53a7c4e9a68093eb9682c8b","role":"drums"}]}','tool','separation-engine',1.0,3,1794656491209,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(4,'mp3-cbr','a1:6f6f98dffb797172f3b4e5497823b82980b251b41a351763d4dc47afecb8c878','intensity','{"audio":{"sample_rate":44100,"seconds":2.038},"bars":null,"cells":{"flux":[1.2383,1.7248,1.4472,1.5752,1.6147,1.434,1.6352,1.3803,1.5718,1.643,0.1541],"high_end":[-40.2,-38.63,-38.56,-38.45,-38.68,-38.75,-38.28,-38.76,-38.6,-38.32,-39.23],"loudness":[-26.79,-26.33,-26.16,-26.43,-26.46,-26.44,-26.05,-26.55,-26.24,-26.23,-27.57],"low_end":[-34.55,-33.69,-33.84,-33.66,-34.55,-34.64,-32.75,-34.3,-32.33,-32.03,-31.06],"onsets":[0,0,0,0,0,0,0,0,0,0,0],"step_seconds":0.2},"curve":{"features":{"high_end":[-38.86,-38.54,-39.23],"loudness":[-26.43,-26.3,-27.57],"low_end":[-34.04,-33.08,-31.06],"onset_density":[0.0,0.0,0.0],"onset_strength":[7.6002,7.6643,4.0553]},"intensity":[0.3818,0.3886,0.3258],"offset_seconds":0.0,"step_seconds":1.0},"grid":null,"method":{"features":["loudness","low_end","high_end","onset_strength","onset_density"],"name":"wreckordbox-intensity","scales":{"high_end":{"ceiling":-20.0,"floor":-60.0},"loudness":{"ceiling":-5.0,"floor":-40.0},"low_end":{"ceiling":-10.0,"floor":-50.0},"onset_density":{"ceiling":8.0,"floor":0.0},"onset_strength":{"ceiling":10.0,"floor":0.0}},"version":2,"weights":{"high_end":0.1,"loudness":0.4,"low_end":0.15,"onset_density":0.2,"onset_strength":0.15}},"summary":{"features":{"high_end":-38.7,"loudness":-26.38,"low_end":-33.47,"onset_density":0.0,"onset_strength":7.5656},"high":0.3886,"intensity":0.3654,"loudness_integrated_lufs":-26.35,"loudness_short_term_max_lufs":-26.41,"low":0.3258}}','tool','wreckordbox-intensity',1.0,4,1794656491231,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(5,'flac','a1:a925e723d25f4748929928c74ecd88990a6941c7392a218d89db002d6bcb0069','stems','{"produced_by":{"engine_version":"synthetic","fixture":"foundation ec1d477d"},"sample_rate":44100,"samples":11025,"stems":[{"bits_per_sample":16,"channels":1,"file":"stems/flac/run-5/vocals.wav","format":"wav","hash":"b3:51fa935c637b275f95e31295010c5a23ecb0c9e10f52fc2d4664623cd86ef7ca","role":"vocals"},{"bits_per_sample":16,"channels":1,"file":"stems/flac/run-5/other.wav","format":"wav","hash":"b3:51fa935c637b275f95e31295010c5a23ecb0c9e10f52fc2d4664623cd86ef7ca","role":"other"},{"bits_per_sample":16,"channels":1,"file":"stems/flac/run-5/bass.wav","format":"wav","hash":"b3:51fa935c637b275f95e31295010c5a23ecb0c9e10f52fc2d4664623cd86ef7ca","role":"bass"},{"bits_per_sample":16,"channels":1,"file":"stems/flac/run-5/drums.wav","format":"wav","hash":"b3:51fa935c637b275f95e31295010c5a23ecb0c9e10f52fc2d4664623cd86ef7ca","role":"drums"}]}','tool','separation-engine',1.0,5,1794656491234,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(6,'flac','a1:a925e723d25f4748929928c74ecd88990a6941c7392a218d89db002d6bcb0069','intensity','{"audio":{"sample_rate":96000,"seconds":0.25},"bars":null,"cells":{"flux":[0.0019,0.0006],"high_end":[-100.0,-100.0],"loudness":[-8.68,-8.67],"low_end":[-91.22,-91.22],"onsets":[0,0],"step_seconds":0.2},"curve":{"features":{"high_end":[-100.0],"loudness":[-8.68],"low_end":[-91.22],"onset_density":[0.0],"onset_strength":[0.01]},"intensity":[0.3581],"offset_seconds":0.0,"step_seconds":1.0},"grid":null,"method":{"features":["loudness","low_end","high_end","onset_strength","onset_density"],"name":"wreckordbox-intensity","scales":{"high_end":{"ceiling":-20.0,"floor":-60.0},"loudness":{"ceiling":-5.0,"floor":-40.0},"low_end":{"ceiling":-10.0,"floor":-50.0},"onset_density":{"ceiling":8.0,"floor":0.0},"onset_strength":{"ceiling":10.0,"floor":0.0}},"version":2,"weights":{"high_end":0.1,"loudness":0.4,"low_end":0.15,"onset_density":0.2,"onset_strength":0.15}},"summary":{"features":{"high_end":-100.0,"loudness":-8.68,"low_end":-91.22,"onset_density":0.0,"onset_strength":0.01},"high":0.3581,"intensity":0.3581,"loudness_integrated_lufs":-8.67,"loudness_short_term_max_lufs":-8.67,"low":0.3581}}','tool','wreckordbox-intensity',1.0,6,1794656491250,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(7,'aiff','a1:20c6864c925b66bf72890349daa1d5a4ccf6287429e4bcffc7552506f76a8526','stems','{"produced_by":{"engine_version":"synthetic","fixture":"foundation ec1d477d"},"sample_rate":44100,"samples":4414,"stems":[{"bits_per_sample":16,"channels":1,"file":"stems/aiff/run-7/vocals.wav","format":"wav","hash":"b3:55fdf94741a80c633f4d818ba9a4c7e80e1db93eae5069e140f9d5ac56885530","role":"vocals"},{"bits_per_sample":16,"channels":1,"file":"stems/aiff/run-7/other.wav","format":"wav","hash":"b3:55fdf94741a80c633f4d818ba9a4c7e80e1db93eae5069e140f9d5ac56885530","role":"other"},{"bits_per_sample":16,"channels":1,"file":"stems/aiff/run-7/bass.wav","format":"wav","hash":"b3:55fdf94741a80c633f4d818ba9a4c7e80e1db93eae5069e140f9d5ac56885530","role":"bass"},{"bits_per_sample":16,"channels":1,"file":"stems/aiff/run-7/drums.wav","format":"wav","hash":"b3:55fdf94741a80c633f4d818ba9a4c7e80e1db93eae5069e140f9d5ac56885530","role":"drums"}]}','tool','separation-engine',1.0,7,1794656491251,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(8,'aiff','a1:20c6864c925b66bf72890349daa1d5a4ccf6287429e4bcffc7552506f76a8526','intensity','{"audio":{"sample_rate":44100,"seconds":0.1},"bars":null,"cells":{"flux":[0.0022],"high_end":[-100.0],"loudness":[-45.55],"low_end":[-9.86],"onsets":[0],"step_seconds":0.2},"curve":{"features":{"high_end":[-100.0],"loudness":[-45.55],"low_end":[-9.86],"onset_density":[0.0],"onset_strength":[0.022]},"intensity":[0.1503],"offset_seconds":0.0,"step_seconds":1.0},"grid":null,"method":{"features":["loudness","low_end","high_end","onset_strength","onset_density"],"name":"wreckordbox-intensity","scales":{"high_end":{"ceiling":-20.0,"floor":-60.0},"loudness":{"ceiling":-5.0,"floor":-40.0},"low_end":{"ceiling":-10.0,"floor":-50.0},"onset_density":{"ceiling":8.0,"floor":0.0},"onset_strength":{"ceiling":10.0,"floor":0.0}},"version":2,"weights":{"high_end":0.1,"loudness":0.4,"low_end":0.15,"onset_density":0.2,"onset_strength":0.15}},"summary":{"features":{"high_end":-100.0,"loudness":-45.55,"low_end":-9.86,"onset_density":0.0,"onset_strength":0.022},"high":0.1503,"intensity":0.1503,"loudness_integrated_lufs":-45.55,"loudness_short_term_max_lufs":-45.55,"low":0.1503}}','tool','wreckordbox-intensity',1.0,8,1794656491252,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(9,'wav','a1:ae9e7a2d397873a12f052eb0c87df0a250591e9c9ff8687dbb00fab7842eac3a','stems','{"produced_by":{"engine_version":"synthetic","fixture":"foundation ec1d477d"},"sample_rate":44100,"samples":4410,"stems":[{"bits_per_sample":16,"channels":1,"file":"stems/wav/run-9/vocals.wav","format":"wav","hash":"b3:30a25d9eeeda5c685d8ed6da56dd1f9ecfa3a319e1c2c31cc5f2da6d4e030aeb","role":"vocals"},{"bits_per_sample":16,"channels":1,"file":"stems/wav/run-9/other.wav","format":"wav","hash":"b3:30a25d9eeeda5c685d8ed6da56dd1f9ecfa3a319e1c2c31cc5f2da6d4e030aeb","role":"other"},{"bits_per_sample":16,"channels":1,"file":"stems/wav/run-9/bass.wav","format":"wav","hash":"b3:30a25d9eeeda5c685d8ed6da56dd1f9ecfa3a319e1c2c31cc5f2da6d4e030aeb","role":"bass"},{"bits_per_sample":16,"channels":1,"file":"stems/wav/run-9/drums.wav","format":"wav","hash":"b3:30a25d9eeeda5c685d8ed6da56dd1f9ecfa3a319e1c2c31cc5f2da6d4e030aeb","role":"drums"}]}','tool','separation-engine',1.0,9,1794656491254,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(10,'wav','a1:ae9e7a2d397873a12f052eb0c87df0a250591e9c9ff8687dbb00fab7842eac3a','intensity','{"audio":{"sample_rate":44100,"seconds":0.1},"bars":null,"cells":{"flux":[0.0022],"high_end":[-100.0],"loudness":[-45.55],"low_end":[-9.86],"onsets":[0],"step_seconds":0.2},"curve":{"features":{"high_end":[-100.0],"loudness":[-45.55],"low_end":[-9.86],"onset_density":[0.0],"onset_strength":[0.022]},"intensity":[0.1503],"offset_seconds":0.0,"step_seconds":1.0},"grid":null,"method":{"features":["loudness","low_end","high_end","onset_strength","onset_density"],"name":"wreckordbox-intensity","scales":{"high_end":{"ceiling":-20.0,"floor":-60.0},"loudness":{"ceiling":-5.0,"floor":-40.0},"low_end":{"ceiling":-10.0,"floor":-50.0},"onset_density":{"ceiling":8.0,"floor":0.0},"onset_strength":{"ceiling":10.0,"floor":0.0}},"version":2,"weights":{"high_end":0.1,"loudness":0.4,"low_end":0.15,"onset_density":0.2,"onset_strength":0.15}},"summary":{"features":{"high_end":-100.0,"loudness":-45.55,"low_end":-9.86,"onset_density":0.0,"onset_strength":0.022},"high":0.1503,"intensity":0.1503,"loudness_integrated_lufs":-45.55,"loudness_short_term_max_lufs":-45.55,"low":0.1503}}','tool','wreckordbox-intensity',1.0,10,1794656491255,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(11,'m4a','a1:5414253530162d57f70ddbcd78a45eb65d5accbcc0e410ff0eb50b75d1b0235e','stems','{"produced_by":{"engine_version":"synthetic","fixture":"foundation ec1d477d"},"sample_rate":44100,"samples":88200,"stems":[{"bits_per_sample":16,"channels":1,"file":"stems/m4a/run-11/vocals.wav","format":"wav","hash":"b3:ef8db0215e785d39a3bb0913b8b0a8d797697423d742ea37a000aeeeb1dc9bb3","role":"vocals"},{"bits_per_sample":16,"channels":1,"file":"stems/m4a/run-11/other.wav","format":"wav","hash":"b3:ef8db0215e785d39a3bb0913b8b0a8d797697423d742ea37a000aeeeb1dc9bb3","role":"other"},{"bits_per_sample":16,"channels":1,"file":"stems/m4a/run-11/bass.wav","format":"wav","hash":"b3:ef8db0215e785d39a3bb0913b8b0a8d797697423d742ea37a000aeeeb1dc9bb3","role":"bass"},{"bits_per_sample":16,"channels":1,"file":"stems/m4a/run-11/drums.wav","format":"wav","hash":"b3:ef8db0215e785d39a3bb0913b8b0a8d797697423d742ea37a000aeeeb1dc9bb3","role":"drums"}]}','tool','separation-engine',1.0,11,1794656491271,NULL,NULL,NULL);
INSERT INTO "fact" VALUES(12,'m4a','a1:5414253530162d57f70ddbcd78a45eb65d5accbcc0e410ff0eb50b75d1b0235e','intensity','{"audio":{"sample_rate":44100,"seconds":2.043},"bars":null,"cells":{"flux":[0.0216,0.0095,0.0098,0.0092,0.0081,0.0074,0.0088,0.0081,0.0088,0.0142,0.1311],"high_end":[-89.41,-100.0,-100.0,-100.0,-100.0,-100.0,-100.0,-100.0,-100.0,-100.0,-80.17],"loudness":[-22.33,-21.76,-21.78,-21.75,-21.76,-21.78,-21.72,-21.76,-21.76,-21.76,-24.46],"low_end":[-57.65,-100.0,-100.0,-100.0,-100.0,-100.0,-100.0,-100.0,-100.0,-94.15,-49.13],"onsets":[0,0,0,0,0,0,0,0,0,0,1],"step_seconds":0.2},"curve":{"features":{"high_end":[-96.4,-100.0,-80.17],"loudness":[-21.87,-21.76,-24.46],"low_end":[-64.64,-100.0,-49.13],"onset_density":[0.0,0.0,23.2558],"onset_strength":[0.0582,0.0473,3.0488]},"intensity":[0.2081,0.2092,0.4266],"offset_seconds":0.0,"step_seconds":1.0},"grid":null,"method":{"features":["loudness","low_end","high_end","onset_strength","onset_density"],"name":"wreckordbox-intensity","scales":{"high_end":{"ceiling":-20.0,"floor":-60.0},"loudness":{"ceiling":-5.0,"floor":-40.0},"low_end":{"ceiling":-10.0,"floor":-50.0},"onset_density":{"ceiling":8.0,"floor":0.0},"onset_strength":{"ceiling":10.0,"floor":0.0}},"version":2,"weights":{"high_end":0.1,"loudness":0.4,"low_end":0.15,"onset_density":0.2,"onset_strength":0.15}},"summary":{"features":{"high_end":-95.02,"loudness":-21.85,"low_end":-63.71,"onset_density":0.4895,"onset_strength":0.1158},"high":0.4266,"intensity":0.2813,"loudness_integrated_lufs":-21.8,"loudness_short_term_max_lufs":-21.91,"low":0.2081}}','tool','wreckordbox-intensity',1.0,12,1794656491294,NULL,NULL,NULL);
CREATE TABLE fact_attachment (
    fact_id INTEGER NOT NULL REFERENCES fact(id),
    change_id INTEGER NOT NULL REFERENCES attachment_change(id),
    PRIMARY KEY(fact_id, change_id)
) STRICT;
CREATE TABLE fact_track (
    fact_id INTEGER PRIMARY KEY REFERENCES fact(id),
    track_id TEXT NOT NULL
) STRICT;
INSERT INTO "fact_track" VALUES(1,'mp3-xing');
INSERT INTO "fact_track" VALUES(2,'mp3-xing');
INSERT INTO "fact_track" VALUES(3,'mp3-cbr');
INSERT INTO "fact_track" VALUES(4,'mp3-cbr');
INSERT INTO "fact_track" VALUES(5,'flac');
INSERT INTO "fact_track" VALUES(6,'flac');
INSERT INTO "fact_track" VALUES(7,'aiff');
INSERT INTO "fact_track" VALUES(8,'aiff');
INSERT INTO "fact_track" VALUES(9,'wav');
INSERT INTO "fact_track" VALUES(10,'wav');
INSERT INTO "fact_track" VALUES(11,'m4a');
INSERT INTO "fact_track" VALUES(12,'m4a');
CREATE TABLE kind (
    name   TEXT PRIMARY KEY,
    nature TEXT NOT NULL CHECK (nature IN ('judgement', 'measurement'))
) STRICT;
INSERT INTO "kind" VALUES('stems','measurement');
INSERT INTO "kind" VALUES('intensity','measurement');
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
CREATE TABLE ledger_intent (
    id          INTEGER PRIMARY KEY REFERENCES ledger_order (seq),
    run_id      INTEGER REFERENCES run (id),
    at          INTEGER NOT NULL,
    source_kind TEXT NOT NULL,
    source_name TEXT NOT NULL,
    description TEXT NOT NULL,
    command     TEXT
) STRICT;
CREATE TABLE ledger_order (
    seq     INTEGER PRIMARY KEY AUTOINCREMENT,
    fact_id INTEGER UNIQUE REFERENCES fact (id)
) STRICT;
INSERT INTO "ledger_order" VALUES(1,1);
INSERT INTO "ledger_order" VALUES(2,2);
INSERT INTO "ledger_order" VALUES(3,3);
INSERT INTO "ledger_order" VALUES(4,4);
INSERT INTO "ledger_order" VALUES(5,5);
INSERT INTO "ledger_order" VALUES(6,6);
INSERT INTO "ledger_order" VALUES(7,7);
INSERT INTO "ledger_order" VALUES(8,8);
INSERT INTO "ledger_order" VALUES(9,9);
INSERT INTO "ledger_order" VALUES(10,10);
INSERT INTO "ledger_order" VALUES(11,11);
INSERT INTO "ledger_order" VALUES(12,12);
CREATE TABLE ledger_owned_run (
    run_id INTEGER PRIMARY KEY REFERENCES run (id)
) STRICT;
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
INSERT INTO "meta" VALUES('schema_version','9');
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
CREATE TABLE mutant_intent (
    id   INTEGER PRIMARY KEY REFERENCES ledger_intent (id) ON DELETE CASCADE,
    plan TEXT NOT NULL
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
INSERT INTO "run" VALUES(1,'tool','separation-engine','Foundation synthetic Stems fixture',1794656491107,1794656491164,'done');
INSERT INTO "run" VALUES(2,'tool','wreckordbox-intensity','Foundation measured Intensity fixture',1794656491164,1794656491192,'done');
INSERT INTO "run" VALUES(3,'tool','separation-engine','Foundation synthetic Stems fixture',1794656491192,1794656491209,'done');
INSERT INTO "run" VALUES(4,'tool','wreckordbox-intensity','Foundation measured Intensity fixture',1794656491209,1794656491232,'done');
INSERT INTO "run" VALUES(5,'tool','separation-engine','Foundation synthetic Stems fixture',1794656491232,1794656491234,'done');
INSERT INTO "run" VALUES(6,'tool','wreckordbox-intensity','Foundation measured Intensity fixture',1794656491234,1794656491250,'done');
INSERT INTO "run" VALUES(7,'tool','separation-engine','Foundation synthetic Stems fixture',1794656491250,1794656491251,'done');
INSERT INTO "run" VALUES(8,'tool','wreckordbox-intensity','Foundation measured Intensity fixture',1794656491251,1794656491253,'done');
INSERT INTO "run" VALUES(9,'tool','separation-engine','Foundation synthetic Stems fixture',1794656491253,1794656491254,'done');
INSERT INTO "run" VALUES(10,'tool','wreckordbox-intensity','Foundation measured Intensity fixture',1794656491254,1794656491255,'done');
INSERT INTO "run" VALUES(11,'tool','separation-engine','Foundation synthetic Stems fixture',1794656491255,1794656491271,'done');
INSERT INTO "run" VALUES(12,'tool','wreckordbox-intensity','Foundation measured Intensity fixture',1794656491271,1794656491294,'done');
CREATE TABLE track_identity (
    track_id TEXT PRIMARY KEY,
    fingerprint TEXT,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    mix_name TEXT NOT NULL,
    duration INTEGER NOT NULL
) STRICT;
CREATE INDEX fact_by_track ON fact (track_id, kind);
CREATE INDEX fact_by_fingerprint ON fact (fingerprint, kind);
CREATE INDEX fact_by_run ON fact (run_id);
CREATE UNIQUE INDEX mutant_live_entry ON mutant (entry_id) WHERE removed_at IS NULL;
CREATE INDEX mutant_by_playlist ON mutant (playlist_id);
CREATE INDEX mutant_by_track ON mutant (track_id);
CREATE INDEX mutant_change_by_mutant ON mutant_change (mutant_id);
CREATE INDEX mutant_change_by_run ON mutant_change (run_id);
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
CREATE TRIGGER fact_takes_its_place_in_order AFTER INSERT ON fact
BEGIN INSERT INTO ledger_order (fact_id) VALUES (new.id); END;
CREATE INDEX fact_by_successor ON fact (superseded_by);
CREATE INDEX ledger_by_run ON ledger (run_id);
CREATE INDEX ledger_by_reversed ON ledger (reverses);
CREATE INDEX ledger_by_whole ON ledger (part_of);
CREATE TRIGGER ledger_is_never_changed BEFORE UPDATE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never changed'); END;
CREATE TRIGGER ledger_is_never_removed BEFORE DELETE ON ledger
BEGIN SELECT RAISE(ABORT, 'the Ledger is append-only: an entry is never removed'); END;
CREATE INDEX identity_audio ON track_identity(fingerprint);
CREATE INDEX identity_fields ON track_identity(title, artist, mix_name, duration);
CREATE INDEX fact_track_by_track ON fact_track(track_id);
CREATE TRIGGER fact_initial_track AFTER INSERT ON fact BEGIN
    INSERT INTO fact_track VALUES (NEW.id, NEW.track_id);
END;
CREATE VIEW attached_fact AS
    SELECT fact.*, fact_track.track_id AS effective_track_id
    FROM fact_track JOIN fact ON fact.id = fact_track.fact_id;
CREATE INDEX run_unfinished ON run (id) WHERE finished_at IS NULL;
CREATE INDEX dj_fact_entry ON dj_fact(ledger_id);
DELETE FROM "sqlite_sequence";
INSERT INTO "sqlite_sequence" VALUES('ledger_order',12);
INSERT INTO "sqlite_sequence" VALUES('run',12);
INSERT INTO "sqlite_sequence" VALUES('fact',12);
COMMIT;
PRAGMA user_version=0;
