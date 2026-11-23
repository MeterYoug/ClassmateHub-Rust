CREATE TABLE seat_subject (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  school_stage TEXT NOT NULL,
  grade TEXT NOT NULL,
  full_score INTEGER NOT NULL CHECK(full_score BETWEEN 1 AND 100000000),
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
  UNIQUE(name, school_stage, grade)
);
CREATE TABLE seat_exam (
  id INTEGER PRIMARY KEY,
  class_id INTEGER NOT NULL REFERENCES seat_class(id),
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  exam_date TEXT NOT NULL DEFAULT '',
  is_current INTEGER NOT NULL DEFAULT 0 CHECK(is_current IN (0, 1)),
  grade_a INTEGER NOT NULL DEFAULT 90,
  grade_b INTEGER NOT NULL DEFAULT 80,
  grade_c INTEGER NOT NULL DEFAULT 70,
  grade_d INTEGER NOT NULL DEFAULT 60,
  CHECK(grade_a <= 100 AND grade_a > grade_b AND grade_b > grade_c AND grade_c > grade_d AND grade_d >= 0),
  UNIQUE(id, class_id)
);
CREATE UNIQUE INDEX exam_current_class ON seat_exam(class_id) WHERE is_current = 1;
CREATE TABLE seat_exam_subject (
  id INTEGER PRIMARY KEY,
  exam_id INTEGER NOT NULL REFERENCES seat_exam(id),
  source_subject_id INTEGER REFERENCES seat_subject(id),
  name TEXT NOT NULL CHECK(length(trim(name)) > 0),
  full_score INTEGER NOT NULL CHECK(full_score BETWEEN 1 AND 100000000),
  counts_toward_total INTEGER NOT NULL DEFAULT 1 CHECK(counts_toward_total IN (0, 1)),
  sort_order INTEGER NOT NULL DEFAULT 0,
  UNIQUE(exam_id, name),
  UNIQUE(id, exam_id)
);
CREATE TABLE seat_student_score (
  exam_id INTEGER NOT NULL,
  student_id INTEGER NOT NULL,
  class_id INTEGER NOT NULL,
  total_score INTEGER CHECK(total_score >= 0),
  total_rank INTEGER CHECK(total_rank > 0),
  total_grade TEXT CHECK(total_grade IN ('A', 'B', 'C', 'D', 'E')),
  PRIMARY KEY(exam_id, student_id),
  FOREIGN KEY(exam_id, class_id) REFERENCES seat_exam(id, class_id),
  FOREIGN KEY(student_id, class_id) REFERENCES seat_student(id, class_id),
  CHECK((total_score IS NULL AND total_rank IS NULL AND total_grade IS NULL) OR
        (total_score IS NOT NULL AND total_rank IS NOT NULL AND total_grade IS NOT NULL))
);
CREATE TABLE seat_student_score_detail (
  exam_id INTEGER NOT NULL,
  student_id INTEGER NOT NULL,
  exam_subject_id INTEGER NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('scored', 'absent')),
  score INTEGER,
  subject_rank INTEGER CHECK(subject_rank > 0),
  subject_grade TEXT CHECK(subject_grade IN ('A', 'B', 'C', 'D', 'E')),
  PRIMARY KEY(exam_id, student_id, exam_subject_id),
  FOREIGN KEY(exam_id, student_id) REFERENCES seat_student_score(exam_id, student_id),
  FOREIGN KEY(exam_subject_id, exam_id) REFERENCES seat_exam_subject(id, exam_id),
  CHECK((status = 'absent' AND score IS NULL AND subject_rank IS NULL AND subject_grade IS NULL) OR
        (status = 'scored' AND score IS NOT NULL AND score >= 0))
);
CREATE INDEX score_detail_subject ON seat_student_score_detail(exam_subject_id, score);
