ALTER TABLE seat_class ADD COLUMN school_year TEXT NOT NULL DEFAULT '';
ALTER TABLE seat_class ADD COLUMN remark TEXT NOT NULL DEFAULT '';
ALTER TABLE seat_student ADD COLUMN score_level TEXT NOT NULL DEFAULT '' CHECK(score_level IN ('', 'A', 'B', 'C', 'D', 'E'));
ALTER TABLE seat_student ADD COLUMN sort_no INTEGER NOT NULL DEFAULT 0 CHECK(sort_no BETWEEN 0 AND 9999);
ALTER TABLE seat_student ADD COLUMN remark TEXT NOT NULL DEFAULT '';
ALTER TABLE seat_exam ADD COLUMN remark TEXT NOT NULL DEFAULT '';
ALTER TABLE seat_exam ADD COLUMN rank_grading INTEGER NOT NULL DEFAULT 0 CHECK(rank_grading IN (0, 1));
ALTER TABLE seat_student_score ADD COLUMN remark TEXT NOT NULL DEFAULT '';
ALTER TABLE seat_plan ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1));

ALTER TABLE seat_position RENAME TO previous_position;
ALTER TABLE seat_classroom RENAME TO previous_classroom;
CREATE TABLE seat_classroom (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL REFERENCES seat_class(id),
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  rows INTEGER NOT NULL CHECK(rows BETWEEN 1 AND 50),
  columns INTEGER NOT NULL CHECK(columns BETWEEN 1 AND 50),
  podium TEXT NOT NULL CHECK(podium IN ('front', 'back', 'left', 'right')),
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
  is_default INTEGER NOT NULL DEFAULT 0 CHECK(is_default IN (0, 1)),
  remark TEXT NOT NULL DEFAULT '',
  UNIQUE(id, class_id)
);
CREATE UNIQUE INDEX classroom_default_class ON seat_classroom(class_id) WHERE is_default = 1 AND active = 1;
CREATE TABLE seat_position (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL,
  classroom_id INTEGER NOT NULL,
  row_number INTEGER NOT NULL CHECK(row_number > 0),
  column_number INTEGER NOT NULL CHECK(column_number > 0),
  kind TEXT NOT NULL CHECK(kind IN ('seat', 'unavailable', 'aisle')),
  desk_group TEXT NOT NULL DEFAULT '',
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
  FOREIGN KEY(classroom_id, class_id) REFERENCES seat_classroom(id, class_id),
  UNIQUE(classroom_id, row_number, column_number)
);
INSERT INTO seat_classroom(id, class_id, name, rows, columns, podium, active)
  SELECT id, class_id, name, rows, columns, podium, active FROM previous_classroom;
INSERT INTO seat_position
  SELECT * FROM previous_position;
DROP TABLE previous_position;
DROP TABLE previous_classroom;

ALTER TABLE seat_rule RENAME TO previous_rule;
CREATE TABLE seat_rule (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL REFERENCES seat_class(id),
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  kind TEXT NOT NULL CHECK(kind IN ('HARD', 'SOFT')),
  weight INTEGER NOT NULL DEFAULT 1 CHECK(weight BETWEEN 0 AND 1000),
  config TEXT NOT NULL DEFAULT '{}',
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0, 1)),
  remark TEXT NOT NULL DEFAULT ''
);
INSERT INTO seat_rule(id, class_id, name, kind, weight, config, active)
  SELECT id, class_id, name, kind, weight, config, active FROM previous_rule;
DROP TABLE previous_rule;
