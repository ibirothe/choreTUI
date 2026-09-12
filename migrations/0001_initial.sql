CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
);

CREATE TABLE chores (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 80),
  description TEXT,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  deleted_at TEXT
);

CREATE TABLE schedules (
  id TEXT PRIMARY KEY,
  chore_id TEXT NOT NULL REFERENCES chores(id),
  kind TEXT NOT NULL CHECK (kind IN ('weekly', 'daily_interval', 'monthly')),
  interval INTEGER NOT NULL CHECK (interval BETWEEN 1 AND 999),
  anchor_date TEXT NOT NULL,
  monthly_day INTEGER CHECK (monthly_day BETWEEN 1 AND 31),
  valid_from TEXT NOT NULL,
  valid_until TEXT,
  created_at TEXT NOT NULL,
  CHECK ((kind = 'monthly') = (monthly_day IS NOT NULL)),
  CHECK (valid_until IS NULL OR valid_until >= valid_from)
);

CREATE UNIQUE INDEX one_active_schedule_per_chore
  ON schedules(chore_id) WHERE valid_until IS NULL;

CREATE TABLE schedule_weekdays (
  schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
  weekday INTEGER NOT NULL CHECK (weekday BETWEEN 1 AND 7),
  PRIMARY KEY (schedule_id, weekday)
);

CREATE TABLE occurrences (
  id TEXT PRIMARY KEY,
  chore_id TEXT NOT NULL REFERENCES chores(id),
  schedule_id TEXT NOT NULL REFERENCES schedules(id),
  nominal_date TEXT NOT NULL,
  due_date TEXT NOT NULL,
  name_snapshot TEXT NOT NULL,
  description_snapshot TEXT,
  state TEXT NOT NULL CHECK (state IN ('pending', 'completed', 'skipped')),
  completed_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (schedule_id, nominal_date),
  CHECK ((state = 'completed') = (completed_at IS NOT NULL))
);

CREATE INDEX occurrences_due_date ON occurrences(due_date);
CREATE INDEX schedules_chore_id ON schedules(chore_id);
