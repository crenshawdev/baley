//! The tables of Figure 10 in design 0001, minus search (slice 8) and the
//! view tables, which are created from each `ViewSpec`.

/// The compatibility epoch this binary writes. A store stamped with a newer
/// one is read-only here; migrations raise it in the transaction that
/// changes the schema (EVD-R19).
pub const EPOCH: u32 = 1;

/// Run once, inside the transaction that finds no `schema_meta`.
pub(crate) const SCHEMA: &str = "
CREATE TABLE schema_meta (
  key TEXT PRIMARY KEY,
  value ANY NOT NULL,
  CHECK (key <> 'epoch' OR typeof(value) = 'integer')
) STRICT, WITHOUT ROWID;

CREATE TABLE project (
  project_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL,
  head_seq INTEGER NOT NULL DEFAULT 0,
  head_hash BLOB CHECK (head_hash IS NULL OR length(head_hash) = 32)
) STRICT, WITHOUT ROWID;

CREATE TABLE checkout (
  project_id TEXT NOT NULL REFERENCES project(project_id),
  path TEXT NOT NULL,
  root_commit TEXT,
  remote_url TEXT,
  last_seen TEXT NOT NULL,
  PRIMARY KEY (project_id, path)
) STRICT, WITHOUT ROWID;

CREATE TABLE anchor (
  project_id TEXT NOT NULL REFERENCES project(project_id),
  seq INTEGER NOT NULL,
  head_hash BLOB NOT NULL CHECK (length(head_hash) = 32),
  tag TEXT NOT NULL,
  pushed_at TEXT NOT NULL,
  PRIMARY KEY (project_id, seq)
) STRICT, WITHOUT ROWID;

CREATE TABLE event (
  project_id TEXT NOT NULL REFERENCES project(project_id),
  seq INTEGER NOT NULL,
  stream TEXT NOT NULL,
  stream_version INTEGER NOT NULL,
  type TEXT NOT NULL,
  type_version INTEGER NOT NULL,
  actor TEXT NOT NULL,
  recorded_at TEXT NOT NULL,
  request_id TEXT NOT NULL,
  git_commit TEXT,
  git_tree TEXT,
  git_checkout TEXT,
  policy_version INTEGER NOT NULL,
  payload_json TEXT NOT NULL,
  prev_hash BLOB CHECK (prev_hash IS NULL OR length(prev_hash) = 32),
  hash BLOB NOT NULL CHECK (length(hash) = 32),
  PRIMARY KEY (project_id, seq)
) STRICT, WITHOUT ROWID;
CREATE UNIQUE INDEX event_stream ON event(project_id, stream, stream_version);
CREATE INDEX event_type ON event(project_id, type, seq);
CREATE INDEX event_commit ON event(project_id, git_commit);

-- A rowid table, not WITHOUT ROWID: incremental blob I/O, which streams a
-- body in chunks, addresses rows by rowid. A reduced or purged row keeps its
-- hash and length and holds its tombstone in place of the body: the excerpt
-- and the ranges it keeps (`kept`, JSON `[[start, end], ...]`), or the
-- reason for the purge.
CREATE TABLE payload (
  hash BLOB PRIMARY KEY CHECK (length(hash) = 32),
  bytes INTEGER NOT NULL CHECK (bytes >= 0),
  encoding TEXT NOT NULL,
  body BLOB,
  state TEXT NOT NULL CHECK (state IN ('present', 'reduced', 'purged')),
  excerpt_hash BLOB REFERENCES payload(hash),
  excerpt_class TEXT CHECK (excerpt_class IN ('record', 'output', 'material')),
  kept TEXT CHECK (json_valid(kept)),
  purge_reason TEXT,
  CHECK ((state = 'present') = (body IS NOT NULL)),
  CHECK ((state = 'reduced') = (excerpt_hash IS NOT NULL)),
  CHECK (excerpt_hash IS NULL OR excerpt_hash <> hash),
  CHECK ((excerpt_hash IS NULL) = (excerpt_class IS NULL)),
  CHECK ((excerpt_hash IS NULL) = (kept IS NULL)),
  CHECK ((state = 'purged') = (purge_reason IS NOT NULL))
) STRICT;
CREATE INDEX payload_excerpt ON payload(excerpt_hash) WHERE excerpt_hash IS NOT NULL;
CREATE INDEX payload_non_present ON payload(state) WHERE state != 'present';

-- The release event sequence is reconstructed from payload.reduced.reference
-- and the [source sequence, hash] pairs in payload.purged.released.
CREATE TABLE payload_ref (
  project_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  hash BLOB NOT NULL REFERENCES payload(hash),
  class TEXT NOT NULL CHECK (class IN ('record', 'output', 'material')),
  expires_at TEXT,
  released_seq INTEGER,
  PRIMARY KEY (project_id, seq, hash),
  FOREIGN KEY (project_id, seq) REFERENCES event(project_id, seq)
) STRICT, WITHOUT ROWID;
CREATE INDEX payload_ref_hash ON payload_ref(hash);

-- Readers and commands use `live_gen`. A rebuild or a view verification
-- replays into `building_gen` and records the last event it applied; the
-- flip moves `live_gen` and clears both in one transaction. A marker left
-- behind belongs to a rebuild or verification that never finished.
CREATE TABLE project_gen (
  project_id TEXT PRIMARY KEY REFERENCES project(project_id),
  live_gen INTEGER NOT NULL,
  building_gen INTEGER,
  building_applied_seq INTEGER,
  CHECK ((building_gen IS NULL) = (building_applied_seq IS NULL))
) STRICT, WITHOUT ROWID;

-- The projector version of each view in each generation, and the view set
-- version of the binary that built it, the same on every row of one
-- generation, so an empty view still says which version built it. Written
-- before the generation's rows, made live with `live_gen`, deleted after
-- them. A zero set version was never stamped and rebuilds forward.
CREATE TABLE view_gen (
  project_id TEXT NOT NULL REFERENCES project(project_id),
  gen INTEGER NOT NULL,
  view TEXT NOT NULL,
  projector_version INTEGER NOT NULL,
  view_set_version INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, gen, view)
) STRICT, WITHOUT ROWID;

-- The registered views of each view set version, sorted and separated by
-- single spaces (view names hold none), so a set changed without a new
-- version is refused at open. Global, like `view_catalog`: it names what a
-- version means, not which project uses it. Version 1 is the store's own
-- `request` view alone.
CREATE TABLE view_set_catalog (
  version INTEGER PRIMARY KEY CHECK (version > 0),
  sorted_view_names TEXT NOT NULL
) STRICT, WITHOUT ROWID;
INSERT INTO view_set_catalog (version, sorted_view_names) VALUES (1, 'request');

-- Each view version's spec as the adapter renders it, so a spec changed
-- without a new version is refused at open instead of read through the
-- table the old spec made.
CREATE TABLE view_catalog (
  view TEXT NOT NULL,
  version INTEGER NOT NULL,
  spec TEXT NOT NULL,
  PRIMARY KEY (view, version)
) STRICT, WITHOUT ROWID;

CREATE TABLE claim_lease (
  project_id TEXT NOT NULL REFERENCES project(project_id),
  kind TEXT NOT NULL,
  request_id TEXT NOT NULL,
  claim_seq INTEGER NOT NULL,
  owner TEXT NOT NULL,
  renewed_at TEXT NOT NULL,
  PRIMARY KEY (project_id, kind, request_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE trace (
  id INTEGER PRIMARY KEY,
  at TEXT NOT NULL,
  project_id TEXT,
  payload_hash BLOB CHECK (payload_hash IS NULL OR length(payload_hash) = 32),
  kind TEXT NOT NULL,
  data TEXT NOT NULL
) STRICT;
CREATE INDEX trace_payload ON trace(payload_hash);
";
