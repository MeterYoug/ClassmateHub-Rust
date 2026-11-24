use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    pub id: i64,
    pub name: String,
    pub full_score: i64,
    pub counts_toward_total: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Exam {
    pub id: i64,
    pub class_id: i64,
    pub name: String,
    pub exam_date: String,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub rank_grading: bool,
    pub is_current: bool,
    pub thresholds: [i64; 4],
    pub subjects: Vec<Subject>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreInput {
    pub student_id: i64,
    pub subject_id: i64,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreRemark {
    pub student_id: i64,
    pub remark: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreRow {
    pub student_id: i64,
    pub student_no: String,
    pub name: String,
    pub active: bool,
    pub total_score: Option<i64>,
    pub total_rank: Option<i64>,
    pub total_grade: Option<String>,
    pub remark: String,
    pub details: Vec<ScoreDetail>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreDetail {
    pub subject_id: i64,
    pub status: String,
    pub score: Option<i64>,
    pub rank: Option<i64>,
    pub grade: Option<String>,
}

fn err(error: impl std::fmt::Display) -> String { error.to_string() }

#[derive(PartialEq, Serialize, Deserialize)]
pub struct SubjectTemplate {
    pub id: i64,
    pub name: String,
    pub school_stage: String,
    pub grade: String,
    pub full_score: i64,
    pub active: bool,
}

pub fn subject_templates(db: &Connection) -> Result<Vec<SubjectTemplate>, String> {
    let mut stmt = db.prepare("SELECT id,name,school_stage,grade,full_score,active FROM seat_subject ORDER BY id").map_err(err)?;
    let rows = stmt.query_map([], |r| Ok(SubjectTemplate { id:r.get(0)?, name:r.get(1)?, school_stage:r.get(2)?, grade:r.get(3)?, full_score:r.get(4)?, active:r.get(5)? })).map_err(err)?;
    rows.collect::<Result<Vec<_>,_>>().map_err(err)
}

pub fn save_subject_template(db: &mut Connection, subject: SubjectTemplate, expected: Option<SubjectTemplate>) -> Result<(), String> {
    if subject.name.trim().is_empty() || !(1..=100_000_000).contains(&subject.full_score) { return Err("科目名称不能为空，满分必须大于零".into()); }
    let db = db.transaction().map_err(err)?;
    if subject.id == 0 {
        db.execute("INSERT INTO seat_subject(name,school_stage,grade,full_score,active) VALUES(?,?,?,?,?)",
            params![subject.name.trim(),subject.school_stage.trim(),subject.grade.trim(),subject.full_score,subject.active]).map_err(err)?;
    } else {
        let current = subject_templates(&db)?.into_iter().find(|item| item.id == subject.id).ok_or("科目模板不存在")?;
        if expected.as_ref() != Some(&current) { return Err("科目模板已被其他操作修改，请重新打开后再编辑".into()); }
        let changed = db.execute("UPDATE seat_subject SET name=?,school_stage=?,grade=?,full_score=?,active=? WHERE id=?",
            params![subject.name.trim(),subject.school_stage.trim(),subject.grade.trim(),subject.full_score,subject.active,subject.id]).map_err(err)?;
        if changed != 1 { return Err("科目模板不存在".into()); }
    }
    db.commit().map_err(err)
}

pub fn exams(db: &Connection, class_id: i64) -> Result<Vec<Exam>, String> {
    let mut query = db.prepare("SELECT id, class_id, name, exam_date, is_current, grade_a, grade_b, grade_c, grade_d, remark, rank_grading FROM seat_exam WHERE class_id=? ORDER BY id").map_err(err)?;
    let rows = query.query_map([class_id], |row| Ok(Exam {
        id: row.get(0)?, class_id: row.get(1)?, name: row.get(2)?, exam_date: row.get(3)?,
        is_current: row.get(4)?, thresholds: [row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?],
        subjects: Vec::new(), remark: row.get(9)?, rank_grading: row.get(10)?,
    })).map_err(err)?;
    let mut exams = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
    let mut query = db.prepare("SELECT id, name, full_score, counts_toward_total FROM seat_exam_subject WHERE exam_id=? ORDER BY sort_order,id").map_err(err)?;
    for exam in &mut exams {
        exam.subjects = query.query_map([exam.id], |row| Ok(Subject {
            id: row.get(0)?, name: row.get(1)?, full_score: row.get(2)?, counts_toward_total: row.get(3)?,
        })).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
    }
    Ok(exams)
}

pub fn save_exam(db: &mut Connection, exam: Exam, expected: Option<Exam>) -> Result<i64, String> {
    let [a, b, c, d] = exam.thresholds;
    if exam.name.trim().is_empty() || !(100 >= a && a > b && b > c && c > d && d >= 0) || exam.subjects.is_empty() {
        return Err("请填写考试名称、至少一个科目及递减的等级阈值".into());
    }
    let mut names = HashSet::new();
    for subject in &exam.subjects {
        if subject.name.trim().is_empty() || !(1..=100_000_000).contains(&subject.full_score) || !names.insert(subject.name.trim()) {
            return Err("科目名称须唯一，满分须大于零".into());
        }
    }
    let tx = db.transaction().map_err(err)?;
    let id = if exam.id == 0 {
        tx.execute("INSERT INTO seat_exam(class_id,name,exam_date,grade_a,grade_b,grade_c,grade_d,remark,rank_grading) VALUES(?,?,?,?,?,?,?,?,?)",
            params![exam.class_id, exam.name.trim(), exam.exam_date, a, b, c, d, exam.remark.trim(), exam.rank_grading]).map_err(err)?;
        let id = tx.last_insert_rowid();
        for (index, subject) in exam.subjects.iter().enumerate() {
            tx.execute("INSERT INTO seat_exam_subject(exam_id,name,full_score,counts_toward_total,sort_order) VALUES(?,?,?,?,?)",
                params![id, subject.name.trim(), subject.full_score, subject.counts_toward_total, index as i64]).map_err(err)?;
        }
        id
    } else {
        let current = exams(&tx, exam.class_id)?.into_iter().find(|item| item.id == exam.id).ok_or("考试不存在或不属于当前班级")?;
        if expected.as_ref() != Some(&current) { return Err("考试已被其他操作修改，请重新打开后再编辑".into()); }
        if current.subjects.len() != exam.subjects.len() || current.subjects.iter().zip(&exam.subjects).any(|(old, new)|
            old.id != new.id || old.name != new.name || old.full_score != new.full_score || old.counts_toward_total != new.counts_toward_total) {
            return Err("考试科目快照不能在此修改".into());
        }
        tx.execute("UPDATE seat_exam SET name=?,exam_date=?,grade_a=?,grade_b=?,grade_c=?,grade_d=?,remark=?,rank_grading=? WHERE id=? AND class_id=?",
            params![exam.name.trim(), exam.exam_date, a,b,c,d,exam.remark.trim(),exam.rank_grading,exam.id,exam.class_id]).map_err(err)?;
        if current.thresholds != exam.thresholds || current.rank_grading != exam.rank_grading {
            recalculate(&tx, exam.id)?;
        }
        exam.id
    };
    if exam.is_current {
        tx.execute("UPDATE seat_exam SET is_current=0 WHERE class_id=?", [exam.class_id]).map_err(err)?;
        tx.execute("UPDATE seat_exam SET is_current=1 WHERE id=?", [id]).map_err(err)?;
    } else {
        tx.execute("UPDATE seat_exam SET is_current=0 WHERE id=?", [id]).map_err(err)?;
    }
    tx.commit().map_err(err)?;
    Ok(id)
}

fn grade(score: i64, full: i64, thresholds: [i64; 4]) -> &'static str {
    for (threshold, label) in thresholds.into_iter().zip(["A", "B", "C", "D"]) {
        if i128::from(score) * 100 >= i128::from(full) * i128::from(threshold) { return label; }
    }
    "E"
}

fn rank_grade(rank: usize, count: usize) -> &'static str {
    if rank <= count.div_ceil(4) { "A" }
    else if rank <= count.div_ceil(2) { "B" }
    else if rank <= (count * 3).div_ceil(4) { "C" }
    else { "D" }
}

pub fn sync_levels(db: &mut Connection, exam_id: i64) -> Result<usize, String> {
    let tx = db.transaction().map_err(err)?;
    let class_id: i64 = tx.query_row("SELECT class_id FROM seat_exam WHERE id=?1", [exam_id], |row| row.get(0)).map_err(err)?;
    // A stale entry changes the quartiles for other pupils in the same exam too.
    let stale_ranking = tx.query_row("SELECT EXISTS(
        SELECT 1 FROM seat_student_score t WHERE t.exam_id=?1 AND t.total_grade IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM seat_student_score_detail d JOIN seat_exam_subject s ON s.id=d.exam_subject_id
                        WHERE d.exam_id=t.exam_id AND d.student_id=t.student_id AND d.status='scored' AND s.counts_toward_total=1)
    )", [exam_id], |row| row.get::<_, bool>(0)).map_err(err)?;
    if stale_ranking {
        return Err("本场旧排名包含未录入计分成绩的学生，请先备份并核对重算本场成绩，再同步等级".into());
    }
    let count = tx.execute("UPDATE seat_student SET score_level=(
        SELECT total_grade FROM seat_student_score WHERE exam_id=?1 AND student_id=seat_student.id)
        WHERE class_id=?2 AND id IN (
        SELECT student_id FROM seat_student_score WHERE exam_id=?1 AND total_grade IS NOT NULL)",
        params![exam_id, class_id]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(count)
}

fn recalculate(tx: &Transaction<'_>, exam_id: i64) -> Result<(), String> {
    let class_id: i64 = tx.query_row("SELECT class_id FROM seat_exam WHERE id=?", [exam_id], |row| row.get(0)).map_err(err)?;
    let exam = exams(tx, class_id)?.into_iter().find(|item| item.id == exam_id).ok_or("考试不存在")?;
    for subject in &exam.subjects {
        let mut stmt = tx.prepare("SELECT student_id,score FROM seat_student_score_detail WHERE exam_id=? AND exam_subject_id=? AND status='scored' ORDER BY score DESC,student_id").map_err(err)?;
        let items = stmt.query_map(params![exam_id, subject.id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
        let mut previous = None;
        let mut rank = 0;
        for (index, (student, score)) in items.into_iter().enumerate() {
            if previous != Some(score) { rank = index as i64 + 1; previous = Some(score); }
            tx.execute("UPDATE seat_student_score_detail SET subject_rank=?,subject_grade=? WHERE exam_id=? AND student_id=? AND exam_subject_id=?",
                params![rank, grade(score, subject.full_score, exam.thresholds), exam_id, student, subject.id]).map_err(err)?;
        }
    }
    let required: Vec<_> = exam.subjects.iter().filter(|s| s.counts_toward_total).collect();
    let full: i64 = required.iter().map(|s| s.full_score).sum();
    let mut stmt = tx.prepare("SELECT student_id FROM seat_student_score WHERE exam_id=? ORDER BY total_rank IS NULL, total_rank, rowid").map_err(err)?;
    let students = stmt.query_map([exam_id], |r| r.get::<_, i64>(0)).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
    let mut totals = Vec::new();
    for student in students {
        let mut total = 0;
        let mut scored_subjects = 0;
        for subject in &required {
            let score: Option<i64> = tx.query_row("SELECT score FROM seat_student_score_detail WHERE exam_id=? AND student_id=? AND exam_subject_id=? AND status='scored'",
                params![exam_id, student, subject.id], |r| r.get(0)).optional().map_err(err)?;
            if let Some(score) = score {
                total += score;
                scored_subjects += 1;
            }
        }
        if scored_subjects > 0 && (exam.rank_grading || scored_subjects == required.len()) { totals.push((student, total)); }
        else {
            tx.execute("UPDATE seat_student_score SET total_score=NULL,total_rank=NULL,total_grade=NULL WHERE exam_id=? AND student_id=?", params![exam_id,student]).map_err(err)?;
        }
    }
    if exam.rank_grading { totals.sort_by_key(|item| std::cmp::Reverse(item.1)); }
    else { totals.sort_by(|a,b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); }
    let count = totals.len();
    let mut previous = None;
    let mut rank = 0;
    for (index, (student, total)) in totals.into_iter().enumerate() {
        if exam.rank_grading || previous != Some(total) { rank = index as i64 + 1; previous = Some(total); }
        let level = if exam.rank_grading { rank_grade(index + 1, count) } else { grade(total,full,exam.thresholds) };
        tx.execute("UPDATE seat_student_score SET total_score=?,total_rank=?,total_grade=? WHERE exam_id=? AND student_id=?",
            params![total,rank,level,exam_id,student]).map_err(err)?;
    }
    Ok(())
}

pub fn parse_score(value: &str, full: i64) -> Result<Option<i64>, String> {
    let value = value.trim();
    if value == "缺考" || value == "清空" { return Ok(None); }
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() > 2 || parts[0].is_empty() || !parts[0].bytes().all(|c| c.is_ascii_digit()) ||
        (parts.len() == 2 && (parts[1].is_empty() || parts[1].len() > 2 || !parts[1].bytes().all(|c| c.is_ascii_digit()))) {
        return Err("分数必须为最多两位小数的非负数字、缺考或清空".into());
    }
    let whole: i64 = parts[0].parse().map_err(err)?;
    let decimal = if parts.len() == 2 { format!("{:0<2}", parts[1]).parse::<i64>().map_err(err)? } else { 0 };
    let score = whole.checked_mul(100).and_then(|n| n.checked_add(decimal)).ok_or("分数过大")?;
    if score > full { return Err("分数超过科目满分".into()); }
    Ok(Some(score))
}

pub fn save_scores(db: &mut Connection, exam_id: i64, input: Vec<ScoreInput>, expected: Option<String>) -> Result<(), String> {
    save_score_changes(db, exam_id, input, vec![], expected)
}

pub fn save_score_changes(db: &mut Connection, exam_id: i64, input: Vec<ScoreInput>, remarks: Vec<ScoreRemark>, expected: Option<String>) -> Result<(), String> {
    let tx = db.transaction().map_err(err)?;
    let class_id: i64 = tx.query_row("SELECT class_id FROM seat_exam WHERE id=?", [exam_id], |r| r.get(0)).map_err(err)?;
    let exam = exams(&tx, class_id)?.into_iter().find(|e| e.id == exam_id).ok_or("考试不存在")?;
    if let Some(expected) = expected {
        if snapshot(&tx, exam_id)? != expected { return Err("考试或学生成绩已变化，请重新预检".into()); }
    }
    let subjects: HashMap<_,_> = exam.subjects.iter().map(|s| (s.id, s.full_score)).collect();
    let mut seen = HashSet::new();
    for item in input {
        if !seen.insert((item.student_id, item.subject_id)) { return Err("学生科目重复".into()); }
        let full = subjects.get(&item.subject_id).ok_or("科目不属于此考试")?;
        let active: Option<i64> = tx.query_row("SELECT active FROM seat_student WHERE id=? AND class_id=?", params![item.student_id,class_id], |r| r.get(0)).optional().map_err(err)?;
        if active != Some(1) { return Err("学生不存在、已停用或不属于当前班级".into()); }
        let value = item.value.trim();
        let parsed = parse_score(value, *full)?;
        tx.execute("INSERT OR IGNORE INTO seat_student_score(exam_id,student_id,class_id) VALUES(?,?,?)", params![exam_id,item.student_id,class_id]).map_err(err)?;
        if value == "清空" {
            tx.execute("DELETE FROM seat_student_score_detail WHERE exam_id=? AND student_id=? AND exam_subject_id=?", params![exam_id,item.student_id,item.subject_id]).map_err(err)?;
            tx.execute("DELETE FROM seat_student_score WHERE exam_id=?1 AND student_id=?2 AND remark=''
                AND NOT EXISTS(SELECT 1 FROM seat_student_score_detail WHERE exam_id=?1 AND student_id=?2)", params![exam_id,item.student_id]).map_err(err)?;
        } else {
            let status = if value == "缺考" { "absent" } else { "scored" };
            tx.execute("INSERT INTO seat_student_score_detail(exam_id,student_id,exam_subject_id,status,score,subject_rank,subject_grade) VALUES(?,?,?,?,?,NULL,NULL) ON CONFLICT(exam_id,student_id,exam_subject_id) DO UPDATE SET status=excluded.status,score=excluded.score,subject_rank=NULL,subject_grade=NULL",
                params![exam_id,item.student_id,item.subject_id,status,parsed]).map_err(err)?;
        }
    }
    for item in remarks {
        let remark = item.remark.trim();
        if remark.chars().count() > 500 { return Err("成绩备注不能超过 500 个字符".into()); }
        let changed = tx.execute("INSERT INTO seat_student_score(exam_id,student_id,class_id,remark)
            SELECT ?1,id,class_id,?4 FROM seat_student WHERE id=?2 AND class_id=?3 AND active=1
            ON CONFLICT(exam_id,student_id) DO UPDATE SET remark=excluded.remark",
            params![exam_id,item.student_id,class_id,remark]).map_err(err)?;
        if changed == 0 { return Err("学生不存在、已停用或不属于当前班级".into()); }
    }
    recalculate(&tx, exam_id)?;
    tx.commit().map_err(err)
}

pub fn remove_score(db: &mut Connection, exam_id: i64, student_id: i64, expected: &str) -> Result<(), String> {
    let tx = db.transaction().map_err(err)?;
    if snapshot(&tx, exam_id)? != expected { return Err("考试或学生成绩已变化，请刷新后重试".into()); }
    tx.execute("DELETE FROM seat_student_score_detail WHERE exam_id=? AND student_id=?", params![exam_id, student_id]).map_err(err)?;
    tx.execute("DELETE FROM seat_student_score WHERE exam_id=? AND student_id=?", params![exam_id, student_id]).map_err(err)?;
    recalculate(&tx, exam_id)?;
    tx.commit().map_err(err)
}

pub fn snapshot(db: &Connection, exam_id: i64) -> Result<String, String> {
    let class_id: i64 = db.query_row("SELECT class_id FROM seat_exam WHERE id=?", [exam_id], |r| r.get(0)).map_err(err)?;
    let exam = exams(db, class_id)?.into_iter().find(|e| e.id == exam_id).ok_or("考试不存在")?;
    serde_json::to_string(&(exam, score_rows(db, exam_id)?)).map_err(err)
}

pub fn score_rows(db: &Connection, exam_id: i64) -> Result<Vec<ScoreRow>, String> {
    let class_id: i64 = db.query_row("SELECT class_id FROM seat_exam WHERE id=?", [exam_id], |r| r.get(0)).map_err(err)?;
    let mut stmt = db.prepare("SELECT s.id,s.student_no,s.name,s.active,t.total_score,t.total_rank,t.total_grade,coalesce(t.remark,'') FROM seat_student s LEFT JOIN seat_student_score t ON t.student_id=s.id AND t.exam_id=? WHERE s.class_id=? AND (s.active=1 OR t.student_id IS NOT NULL) ORDER BY s.student_no,s.id").map_err(err)?;
    let mut rows = stmt.query_map(params![exam_id,class_id], |r| Ok(ScoreRow { student_id:r.get(0)?, student_no:r.get(1)?,name:r.get(2)?,active:r.get(3)?,total_score:r.get(4)?,total_rank:r.get(5)?,total_grade:r.get(6)?,remark:r.get(7)?,details:Vec::new() })).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    let mut details = db.prepare("SELECT exam_subject_id,status,score,subject_rank,subject_grade FROM seat_student_score_detail WHERE exam_id=? AND student_id=?").map_err(err)?;
    for row in &mut rows {
        row.details = details.query_map(params![exam_id,row.student_id], |r| Ok(ScoreDetail { subject_id:r.get(0)?,status:r.get(1)?,score:r.get(2)?,rank:r.get(3)?,grade:r.get(4)? })).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_grading_requires_a_scored_counting_subject_and_accepts_zero() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::database::initialize(&mut db).unwrap();
        db.execute_batch("INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(1,'测试班','','','');
            INSERT INTO seat_student(id,class_id,student_no,name,score_level) VALUES
            (1,1,'01','甲','D'), (2,1,'02','乙',''), (3,1,'03','丙',''), (4,1,'04','丁','');
            INSERT INTO seat_exam(id,class_id,name,rank_grading) VALUES(1,1,'测试',1);
            INSERT INTO seat_exam_subject(id,exam_id,name,full_score,counts_toward_total)
            VALUES(1,1,'数学',10000,1),(2,1,'语文',10000,1),(3,1,'选修',10000,0)").unwrap();
        let mark = |student_id, subject_id, value: &str| ScoreInput { student_id, subject_id, value: value.into() };
        let marks = vec![mark(2,1,"0"), mark(3,1,"缺考"), mark(4,3,"90")];
        let remark = |value: &str| vec![ScoreRemark { student_id: 1, remark: value.into() }];
        save_score_changes(&mut db, 1, marks, remark("待补录"), None).unwrap();
        save_score_changes(&mut db, 1, vec![], remark(""), None).unwrap();
        let totals = |db: &Connection| score_rows(db, 1).unwrap().into_iter()
            .map(|r| (r.total_score, r.total_rank, r.total_grade)).collect::<Vec<_>>();
        assert_eq!(totals(&db), vec![(None,None,None),(Some(0),Some(1),Some("A".into())),(None,None,None),(None,None,None)]);
        assert_eq!(sync_levels(&mut db, 1).unwrap(), 1);
        // One stale grade can change the rankings of all eligible students.
        db.execute("UPDATE seat_student_score SET total_score=0,total_rank=2,total_grade='C' WHERE student_id=3", []).unwrap();
        assert!(sync_levels(&mut db, 1).unwrap_err().contains("旧排名"));
        assert_eq!(crate::database::list(&db, "students", Some(1)).unwrap()[0]["score_level"], "D");
        save_scores(&mut db, 1, vec![mark(1,1,"90")], None).unwrap();
        assert_eq!(totals(&db)[1].1, Some(2));
        save_scores(&mut db, 1, vec![mark(1,1,"清空")], None).unwrap();
        assert_eq!(totals(&db)[0], (None,None,None));
        assert_eq!(totals(&db)[1].1, Some(1));
    }

    #[test]
    fn scores_merge_rank_absence_and_reject_stale_imports() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::database::initialize(&mut db).unwrap();
        db.execute("INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(1,'测试班','','','')", []).unwrap();
        for id in 1..=4 {
            db.execute("INSERT INTO seat_student(id,class_id,student_no,name) VALUES(?,1,?,?)", params![id,format!("{id:03}"),format!("学生{id}")]).unwrap();
        }
        let exam = Exam { id:0,class_id:1,name:"考试".into(),exam_date:"".into(),remark:String::new(),rank_grading:false,is_current:true,thresholds:[90,80,70,60],
            subjects: vec![Subject{id:0,name:"语文".into(),full_score:10000,counts_toward_total:true},
                Subject{id:0,name:"数学".into(),full_score:10000,counts_toward_total:true}] };
        let id = save_exam(&mut db, exam, None).unwrap();
        let exam = exams(&db, 1).unwrap().remove(0);
        let (a,b) = (exam.subjects[0].id, exam.subjects[1].id);
        let first = (1..=4).map(|student_id| ScoreInput { student_id, subject_id:a, value:if student_id == 4 { "缺考" } else { "90" }.into() }).collect();
        save_scores(&mut db,id,first,None).unwrap();
        let before = snapshot(&db,id).unwrap();
        let input = vec![ScoreInput{student_id:1,subject_id:b,value:"100".into()},
            ScoreInput{student_id:2,subject_id:b,value:"100".into()},
            ScoreInput{student_id:3,subject_id:b,value:"80".into()},
            ScoreInput{student_id:4,subject_id:b,value:"100".into()}];
        save_scores(&mut db,id,input,Some(before.clone())).unwrap();
        let rows = score_rows(&db,id).unwrap();
        assert_eq!(rows.iter().map(|r| r.total_rank).collect::<Vec<_>>(), vec![Some(1),Some(1),Some(3),None]);
        assert_eq!(rows[0].total_grade.as_deref(),Some("A"));
        assert_eq!(rows[3].details.iter().find(|d| d.subject_id==a).unwrap().status,"absent");
        assert!(save_scores(&mut db,id,vec![ScoreInput{student_id:1,subject_id:b,value:"清空".into()}],Some(before)).is_err());
        assert_eq!(score_rows(&db,id).unwrap()[0].total_score,Some(19000));
        let invalid = vec![ScoreInput{student_id:1,subject_id:b,value:"0".into()}, ScoreInput{student_id:2,subject_id:b,value:"101".into()}];
        assert!(save_scores(&mut db,id,invalid,None).is_err());
        assert_eq!(score_rows(&db,id).unwrap()[0].total_score,Some(19000));
        save_scores(&mut db,id,vec![ScoreInput{student_id:1,subject_id:b,value:"清空".into()}],None).unwrap();
        assert_eq!(score_rows(&db,id).unwrap()[0].total_rank,None);
        assert!(parse_score("0.001",10000).is_err());
        assert_eq!(parse_score("0.1",10000).unwrap(),Some(10));
    }
}
