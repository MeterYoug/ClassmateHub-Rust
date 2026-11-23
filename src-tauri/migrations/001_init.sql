CREATE TABLE seat_class (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  school_stage TEXT NOT NULL,
  grade TEXT NOT NULL,
  term TEXT NOT NULL,
  subjects TEXT NOT NULL DEFAULT '',
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1))
);
CREATE TABLE seat_student (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL REFERENCES seat_class(id),
  student_no TEXT NOT NULL CHECK(length(trim(student_no)) > 0),
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  gender TEXT NOT NULL DEFAULT '',
  height REAL CHECK(height IS NULL OR height > 0),
  eyesight TEXT NOT NULL DEFAULT '',
  discipline TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT '',
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
  UNIQUE(id, class_id)
);
CREATE UNIQUE INDEX student_active_number ON seat_student(class_id, student_no) WHERE active = 1;
CREATE TABLE seat_classroom (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL REFERENCES seat_class(id),
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  rows INTEGER NOT NULL CHECK(rows BETWEEN 1 AND 50),
  columns INTEGER NOT NULL CHECK(columns BETWEEN 1 AND 50),
  podium TEXT NOT NULL CHECK(podium IN ('front', 'back')),
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
  UNIQUE(id, class_id)
);
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
CREATE TABLE seat_rule (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL REFERENCES seat_class(id),
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  kind TEXT NOT NULL CHECK(kind IN ('HARD', 'SOFT')),
  weight INTEGER NOT NULL DEFAULT 1 CHECK(weight BETWEEN 0 AND 100),
  config TEXT NOT NULL DEFAULT '{}',
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1))
);
CREATE TABLE seat_student_relation (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL,
  student_id INTEGER NOT NULL,
  other_student_id INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('NOT_DESKMATE', 'NOT_ADJACENT', 'PREFER_DESKMATE')),
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
  FOREIGN KEY(student_id, class_id) REFERENCES seat_student(id, class_id),
  FOREIGN KEY(other_student_id, class_id) REFERENCES seat_student(id, class_id),
  CHECK(student_id < other_student_id),
  UNIQUE(class_id, student_id, other_student_id, kind)
);
