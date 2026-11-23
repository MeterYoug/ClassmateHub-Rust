CREATE TABLE seat_plan (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL,
  name TEXT NOT NULL CHECK(length(trim(name)) BETWEEN 1 AND 100),
  status TEXT NOT NULL CHECK(status IN ('draft', 'confirmed')),
  revision INTEGER NOT NULL CHECK(revision >= 1),
  snapshot TEXT NOT NULL CHECK(json_valid(snapshot)),
  seed INTEGER NOT NULL CHECK(seed BETWEEN 0 AND 4294967295),
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX seat_plan_class ON seat_plan(class_id, id DESC);

CREATE TABLE seat_assignment (
  plan_id INTEGER NOT NULL REFERENCES seat_plan(id),
  student_id INTEGER NOT NULL,
  position_id INTEGER NOT NULL,
  locked INTEGER NOT NULL CHECK(locked IN (0, 1)),
  PRIMARY KEY(plan_id, student_id),
  UNIQUE(plan_id, position_id)
);

CREATE TABLE seat_plan_score (
  plan_id INTEGER NOT NULL REFERENCES seat_plan(id),
  rule_order INTEGER NOT NULL CHECK(rule_order >= 0),
  label TEXT NOT NULL,
  score INTEGER NOT NULL CHECK(score >= 0),
  maximum INTEGER NOT NULL CHECK(maximum >= score),
  PRIMARY KEY(plan_id, rule_order)
);
