use std::{collections::{HashMap, HashSet}, io::Cursor};
use calamine::{Data, Reader, Xlsx};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use crate::scores::{self, ScoreInput, ScoreRemark};

fn err(value: impl std::fmt::Display) -> String { value.to_string() }

#[derive(Serialize, Deserialize)]
pub struct ScorePreview {
    pub errors: Vec<String>,
    pub changes: Vec<ScoreInput>,
    pub remarks: Vec<ScoreRemark>,
    pub snapshot: String,
    pub additions: usize,
    pub replacements: usize,
    pub clears: usize,
}

fn sheet(bytes: Vec<u8>) -> Result<Vec<Vec<String>>, String> {
    if bytes.len() > 10 * 1024 * 1024 { return Err("文件超过 10 MB".into()); }
    let mut workbook: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).map_err(err)?;
    let first = workbook.sheet_names().first().cloned().ok_or("工作簿没有工作表")?;
    let range = workbook.worksheet_range(&first).map_err(err)?;
    if range.height() > 10001 || range.width() > 200 { return Err("表格超过 10000 行或 200 列".into()); }
    for (index, row) in range.rows().enumerate().skip(1) {
        if let Some(cell) = row.first() {
            if !matches!(cell, Data::String(_) | Data::Empty) { return Err(format!("第 {} 行：学号必须为文本格式", index + 1)); }
        }
    }
    Ok(range.rows().map(|row| row.iter().map(|cell| match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Int(n) => n.to_string(),
        Data::Float(n) => n.to_string(),
        other => other.to_string(),
    }).collect()).collect())
}

pub fn score_preview(db: &Connection, exam_id: i64, bytes: Vec<u8>) -> Result<ScorePreview, String> {
    let class_id: i64 = db.query_row("SELECT class_id FROM seat_exam WHERE id=?", [exam_id], |r| r.get(0)).map_err(err)?;
    let exam = scores::exams(db, class_id)?.into_iter().find(|e| e.id == exam_id).ok_or("考试不存在")?;
    let students = scores::score_rows(db, exam_id)?;
    let student_ids: HashMap<_,_> = students.iter().filter(|s| s.active).map(|s| (s.student_no.as_str(), s)).collect();
    let rows = sheet(bytes)?;
    let header = rows.first().ok_or("表格为空")?;
    let mut errors = Vec::new();
    if header.first().map(String::as_str) != Some("学号") { errors.push("第 1 列表头必须为学号".into()); }
    let mut names = HashSet::new();
    let mut columns = Vec::new();
    let mut name_column = None;
    let mut remark_column = None;
    for (index, name) in header.iter().enumerate().skip(1) {
        if name.is_empty() {
            if rows.iter().skip(1).any(|row| row.get(index).is_some_and(|value| !value.is_empty())) {
                errors.push(format!("第 1 行第 {} 列：有数据的列缺少科目名称", index + 1));
            }
            continue;
        }
        if !names.insert(name) { errors.push(format!("第 1 行第 {} 列：科目重复", index + 1)); }
        if name == "姓名" || name == "学生姓名" {
            name_column = Some(index);
            continue;
        }
        if name == "备注" { remark_column = Some(index); continue; }
        if matches!(name.as_str(), "总分" | "班级排名" | "成绩等级" | "等级") {
            continue;
        }
        match exam.subjects.iter().find(|s| &s.name == name) {
            Some(subject) => columns.push((index, subject)),
            None => errors.push(format!("第 1 行第 {} 列：未知科目 {}", index + 1, name)),
        }
    }
    if columns.is_empty() { errors.push("至少需要一个考试科目列".into()); }
    let mut seen = HashSet::new();
    let mut changes = Vec::new();
    let mut remarks = Vec::new();
    for (index, row) in rows.iter().enumerate().skip(1) {
        if row.iter().all(String::is_empty) { continue; }
        let number = row.first().map(String::as_str).unwrap_or("");
        if !seen.insert(number.to_string()) { errors.push(format!("第 {} 行第 1 列：学号重复", index + 1)); continue; }
        let student = match student_ids.get(number) {
            Some(student) => *student,
            None => { errors.push(format!("第 {} 行第 1 列：学号不存在或学生已停用", index + 1)); continue; }
        };
        if let Some(name) = name_column.and_then(|index| row.get(index)).filter(|name| !name.is_empty()) {
            if name != &student.name {
                errors.push(format!("第 {} 行：学号与姓名不匹配", index + 1));
                continue;
            }
        }
        if let Some(remark) = remark_column.and_then(|index| row.get(index)).filter(|remark| !remark.is_empty()) {
            if remark.chars().count() <= 500 {
                remarks.push(ScoreRemark { student_id: student.student_id, remark: remark.clone() });
            } else {
                errors.push(format!("第 {} 行：备注不能超过 500 个字符", index + 1));
            }
        }
        for (column, subject) in &columns {
            let value = row.get(*column).map(String::as_str).unwrap_or("");
            if value.is_empty() { continue; }
            match scores::parse_score(value, subject.full_score) {
                Ok(_) => changes.push(ScoreInput { student_id: student.student_id, subject_id: subject.id, value: value.to_string() }),
                Err(error) => errors.push(format!("第 {} 行第 {} 列：{}", index + 1, column + 1, error)),
            }
        }
    }
    let mut additions = 0;
    let mut replacements = 0;
    let mut clears = 0;
    for change in &changes {
        let existing = students.iter().find(|s| s.student_id == change.student_id)
            .is_some_and(|s| s.details.iter().any(|d| d.subject_id == change.subject_id));
        if change.value == "清空" { clears += 1; }
        else if existing { replacements += 1; }
        else { additions += 1; }
    }
    Ok(ScorePreview { errors, changes, remarks, snapshot: scores::snapshot(db, exam_id)?, additions, replacements, clears })
}

pub fn score_template(db: &Connection, exam_id: i64) -> Result<Vec<u8>, String> {
    score_workbook(db, exam_id, false, true)
}

pub fn score_workbook(db: &Connection, exam_id: i64, export: bool, by_student_no: bool) -> Result<Vec<u8>, String> {
    let class_id: i64 = db.query_row("SELECT class_id FROM seat_exam WHERE id=?", [exam_id], |r| r.get(0)).map_err(err)?;
    let exam = scores::exams(db, class_id)?.into_iter().find(|e| e.id == exam_id).ok_or("考试不存在")?;
    let mut students = scores::score_rows(db, exam_id)?;
    if !by_student_no { students.sort_by_key(|student| student.total_rank.unwrap_or(i64::MAX)); }
    else {
        students.sort_by(|left, right| {
            fn suffix(number: &str) -> Option<&str> {
                let start = number.trim_end_matches(|char: char| char.is_ascii_digit()).len();
                (start < number.len()).then(|| &number[start..])
            }
            match (suffix(&left.student_no), suffix(&right.student_no)) {
                (Some(left_suffix), Some(right_suffix)) => {
                    let a = left_suffix.trim_start_matches('0');
                    let b = right_suffix.trim_start_matches('0');
                    a.len().cmp(&b.len()).then(a.cmp(b)).then(left.student_no.cmp(&right.student_no))
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left.student_no.cmp(&right.student_no),
            }
        });
    }
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    let mut headers = vec!["学号", "姓名"];
    headers.extend(exam.subjects.iter().map(|subject| subject.name.as_str()));
    headers.extend(["总分", "班级排名", "备注"]);
    if export { headers.push("成绩等级"); }
    for (column, title) in headers.into_iter().enumerate() {
        sheet.write_string(0, column as u16, title).map_err(err)?;
    }
    for (index, student) in students.iter().filter(|s| if export { !s.details.is_empty() || !s.remark.is_empty() } else { s.active }).enumerate() {
        let row = (index + 1) as u32;
        sheet.write_string(row, 0, &student.student_no).map_err(err)?;
        sheet.write_string(row, 1, &student.name).map_err(err)?;
        if !export { continue; }
        for (index, subject) in exam.subjects.iter().enumerate() {
            let column = (index + 2) as u16;
            if let Some(detail) = student.details.iter().find(|detail| detail.subject_id == subject.id) {
                if let Some(score) = detail.score { sheet.write_number(row, column, score as f64 / 100.0).map_err(err)?; }
                else if detail.status == "absent" { sheet.write_string(row, column, "缺考").map_err(err)?; }
            }
        }
        let total = (exam.subjects.len() + 2) as u16;
        if let Some(value) = student.total_score { sheet.write_number(row, total, value as f64 / 100.0).map_err(err)?; }
        if let Some(rank) = student.total_rank { sheet.write_number(row, total + 1, rank as f64).map_err(err)?; }
        sheet.write_string(row, total + 2, &student.remark).map_err(err)?;
        if let Some(grade) = &student.total_grade { sheet.write_string(row, total + 3, grade).map_err(err)?; }
    }
    book.save_to_buffer().map_err(err)
}

pub type StudentImport = serde_json::Map<String, serde_json::Value>;

const STUDENT_COLUMNS: &[(&str, &str)] = &[
    ("学号", "student_no"), ("学生姓名", "name"), ("性别", "gender"), ("身高厘米", "height"),
    ("视力等级", "eyesight"), ("成绩等级", "score_level"), ("纪律等级", "discipline"),
    ("特殊需求", "notes"), ("排序号", "sort_no"), ("备注", "remark"),
];

#[derive(Serialize)]
pub struct StudentPreview {
    pub errors: Vec<String>,
    pub students: Vec<StudentImport>,
    pub snapshot: String,
}

fn student_snapshot(db: &Connection, class_id: i64) -> Result<String, String> {
    let class = crate::database::list(db, "classes", None)?.into_iter().find(|c| c["id"].as_i64() == Some(class_id)).ok_or("班级不存在")?;
    serde_json::to_string(&(class, crate::database::list(db, "students", Some(class_id))?)).map_err(err)
}

pub fn student_preview(db: &Connection, class_id: i64, bytes: Vec<u8>, update_existing: bool) -> Result<StudentPreview, String> {
    let snapshot = student_snapshot(db, class_id)?;
    let rows = sheet(bytes)?;
    let header = rows.first().ok_or("表格为空")?;
    if header.first().map(String::as_str) != Some("学号") { return Err("第 1 列表头必须为学号".into()); }
    let mut columns = HashSet::new();
    let keys = header.iter().map(|title| {
        let title = if title == "姓名" { "学生姓名" } else { title };
        let (_, key) = STUDENT_COLUMNS.iter().find(|(label, _)| *label == title).ok_or_else(|| format!("未知学生表头：{title}"))?;
        if !columns.insert(*key) { return Err(format!("学生表头重复：{title}")); }
        Ok(*key)
    }).collect::<Result<Vec<_>, String>>()?;
    let name_index = keys.iter().position(|key| *key == "name").ok_or("缺少学生姓名列")?;
    let existing = crate::database::list(db, "students", Some(class_id))?;
    let active: HashSet<_> = existing.iter().filter(|s| s["active"] == 1).filter_map(|s| s["student_no"].as_str()).collect();
    let mut seen = HashSet::new();
    let mut errors = Vec::new();
    let mut students = Vec::new();
    for (index, row) in rows.iter().enumerate().skip(1) {
        if row.iter().all(String::is_empty) { continue; }
        let number = row.first().map(String::as_str).unwrap_or("");
        let name = row.get(name_index).map(String::as_str).unwrap_or("");
        if number.is_empty() { errors.push(format!("第 {} 行第 1 列：学号不能为空", index + 1)); continue; }
        if name.is_empty() { errors.push(format!("第 {} 行第 2 列：姓名不能为空", index + 1)); continue; }
        if !seen.insert(number.to_string()) || (!update_existing && active.contains(number)) { errors.push(format!("第 {} 行第 1 列：学号重复或已存在", index + 1)); continue; }
        let mut student = StudentImport::new();
        for (&key, value) in keys.iter().zip(row) {
            if value.is_empty() { continue; }
            match student_value(key, value) {
                Ok(value) => { student.insert(key.into(), value); }
                Err(error) => errors.push(format!("第 {} 行：{error}", index + 1)),
            }
        }
        students.push(student);
    }
    Ok(StudentPreview { errors, students, snapshot })
}

fn student_value(key: &str, value: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    let label = match (key, value) {
        ("gender", "0" | "男") => "男",
        ("gender", "1" | "女") => "女",
        ("gender", "2" | "未知") => "未知",
        ("eyesight", "0" | "正常") => "正常",
        ("eyesight", "1" | "轻度") => "轻度",
        ("eyesight", "2" | "中度") => "中度",
        ("eyesight", "3" | "重度") => "重度",
        ("discipline", "0" | "正常") => "正常",
        ("discipline", "1" | "关注") => "关注",
        ("discipline", "2" | "重点关注") => "重点关注",
        ("score_level", "A" | "B" | "C" | "D" | "E") => value,
        ("height", value) => {
            let value: f64 = value.parse().map_err(|_| "身高须为 50 至 250 厘米")?;
            if !(50.0..=250.0).contains(&value) { return Err("身高须为 50 至 250 厘米".into()); }
            return Ok(Value::from(value));
        }
        ("sort_no", value) => {
            let value: i64 = value.parse().map_err(|_| "排序号须为 0 至 9999 的整数")?;
            if !(0..=9999).contains(&value) { return Err("排序号须为 0 至 9999 的整数".into()); }
            return Ok(Value::from(value));
        }
        ("gender" | "eyesight" | "discipline" | "score_level", _) => {
            let (label, _) = STUDENT_COLUMNS.iter().find(|(_, field)| *field == key).unwrap();
            return Err(format!("{label}选项无效：{value}"));
        }
        _ => value,
    };
    Ok(Value::from(label))
}

pub fn save_students(db: &mut Connection, class_id: i64, students: Vec<StudentImport>, expected: String, update_existing: bool) -> Result<(), String> {
    let tx = db.transaction().map_err(err)?;
    if student_snapshot(&tx, class_id)? != expected { return Err("班级或学生名单已变化，请重新预检".into()); }
    let records = crate::database::list(&tx, "students", Some(class_id))?;
    let mut seen = HashSet::new();
    for student in students {
        let mut validated = StudentImport::new();
        for (key, value) in student {
            if !STUDENT_COLUMNS.iter().any(|(_, field)| *field == key) { return Err("学生表字段无效".into()); }
            let text = match value {
                serde_json::Value::String(text) => text.trim().to_string(),
                serde_json::Value::Number(value) if matches!(key.as_str(), "height" | "sort_no") => value.to_string(),
                _ => return Err("学生表字段类型无效".into()),
            };
            validated.insert(key.clone(), student_value(&key, &text)?);
        }
        let number = validated.get("student_no").and_then(serde_json::Value::as_str).filter(|value| !value.is_empty()).ok_or("学号不能为空")?;
        if !seen.insert(number.to_string()) { return Err("学生导入表学号重复".into()); }
        let mut record = if let Some(current) = records.iter().find(|record| record["active"] == 1 && record["student_no"] == number) {
            if !update_existing { return Err(format!("学号 {number} 已存在")); }
            current.as_object().cloned().ok_or("学生记录无效")?
        } else {
            serde_json::json!({"class_id":class_id,"gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":1})
                .as_object().cloned().ok_or("学生记录无效")?
        };
        record.extend(validated);
        crate::database::save(&tx, "students", record)?;
    }
    tx.commit().map_err(err)
}

pub fn student_template() -> Result<Vec<u8>, String> {
    records_workbook("students", &[])
}

pub fn records_workbook(resource: &str, records: &[serde_json::Value]) -> Result<Vec<u8>, String> {
    let columns = match resource {
        "students" => STUDENT_COLUMNS,
        "classes" => &[("班级名称", "name"), ("学段", "school_stage"), ("年级", "grade"), ("学年", "school_year"),
            ("学期", "term"), ("科目", "subjects"), ("状态", "active"), ("备注", "remark")],
        "classrooms" => &[("教室布局名称", "name"), ("座位行数", "rows"), ("座位列数", "columns"),
            ("讲台位置", "podium"), ("默认布局", "is_default"), ("状态", "active"), ("备注", "remark")],
        "rules" => &[("规则名称", "name"), ("规则类别", "kind"), ("规则权重", "weight"),
            ("规则配置", "config"), ("是否启用", "enabled"), ("状态", "active"), ("备注", "remark")],
        _ => return Err("该资料类型不支持导出".into()),
    };
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    for (column, (label, field)) in columns.iter().enumerate() {
        sheet.write_string(0, column as u16, *label).map_err(err)?;
        for (index, record) in records.iter().enumerate() {
            let row = (index + 1) as u32;
            let value = &record[*field];
            if let Some(text) = value.as_str() {
                sheet.write_string(row, column as u16, text).map_err(err)?;
            } else if let Some(number) = value.as_f64() {
                sheet.write_number(row, column as u16, number).map_err(err)?;
            }
        }
    }
    book.save_to_buffer().map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scores::{Exam, Subject};

    fn workbook(rows: &[&[&str]]) -> Vec<u8> {
        let mut book = rust_xlsxwriter::Workbook::new();
        let sheet = book.add_worksheet();
        for (row, values) in rows.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                sheet.write_string(row as u32, column as u16, *value).unwrap();
            }
        }
        book.save_to_buffer().unwrap()
    }

    #[test]
    fn original_workbooks_keep_attributes_and_recalculate_rank_levels() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::database::initialize(&mut db).unwrap();
        db.execute("INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(1,'测试班','','','')", []).unwrap();
        let students = workbook(&[&["学号","学生姓名","性别","身高厘米","视力等级","纪律等级"],
            &["001","测试甲","男","151.5","轻度","关注"], &["002","测试乙","女","160","中度","正常"]]);
        let preview = student_preview(&db, 1, students, false).unwrap();
        assert!(preview.errors.is_empty());
        save_students(&mut db, 1, preview.students, preview.snapshot, false).unwrap();
        let update = workbook(&[&["学号","姓名","排序号"], &["001","测试甲","9"]]);
        let preview = student_preview(&db, 1, update.clone(), false).unwrap();
        assert_eq!(preview.errors.len(), 1);
        let preview = student_preview(&db, 1, update, true).unwrap();
        assert!(preview.errors.is_empty());
        save_students(&mut db, 1, preview.students, preview.snapshot, true).unwrap();
        let records = crate::database::list(&db, "students", Some(1)).unwrap();
        let student = &records.iter().find(|item| item["student_no"] == "001").unwrap();
        assert_eq!(student["sort_no"], 9);
        assert_eq!(student["eyesight"], "轻度");
        assert_eq!(student["discipline"], "关注");
        assert_eq!(student["height"], 151.5);

        let exam = Exam { id:0,class_id:1,name:"考试".into(),exam_date:"".into(),remark:String::new(),
            rank_grading:true,is_current:true,thresholds:[90,80,70,60],
            subjects:vec![Subject{id:0,name:"数学".into(),full_score:15000,counts_toward_total:true}] };
        let id = scores::save_exam(&mut db, exam, None).unwrap();
        let scores = workbook(&[&["学号","姓名","数学","总分","班级排名","备注"],
            &["002","测试乙","120","0","2","旧表备注"], &["001","测试甲","120","0","1",""]]);
        let preview = score_preview(&db, id, scores).unwrap();
        assert!(preview.errors.is_empty());
        scores::save_score_changes(&mut db, id, preview.changes, preview.remarks, Some(preview.snapshot)).unwrap();
        let rows = scores::score_rows(&db, id).unwrap();
        assert_eq!((rows[1].total_score, rows[1].total_rank, rows[1].total_grade.as_deref()), (Some(12000), Some(1), Some("A")));
        assert_eq!((rows[0].total_rank, rows[0].total_grade.as_deref()), (Some(2), Some("C")));
        assert_eq!(rows[1].remark, "旧表备注");
        assert_eq!(scores::sync_levels(&mut db, id).unwrap(), 2);
        assert_eq!(crate::database::list(&db, "students", Some(1)).unwrap()[0]["score_level"], "A");
        let exported = sheet(score_workbook(&db, id, true, true).unwrap()).unwrap();
        assert_eq!(&exported[1][..3], ["001","测试甲","120"]);
        assert_eq!(&exported[2][5], "旧表备注");
    }

    #[test]
    fn import_previews_report_errors_and_detect_changed_data() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::database::initialize(&mut db).unwrap();
        db.execute("INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(1,'测试班','','','')", []).unwrap();
        let invalid = workbook(&[&["学号","姓名"], &["001","测试甲"], &["001","测试乙"]]);
        let preview = student_preview(&db, 1, invalid, false).unwrap();
        assert_eq!(preview.errors.len(), 1);
        assert_eq!(crate::database::list(&db,"students",Some(1)).unwrap().len(), 0);
        let valid = workbook(&[&["学号","姓名"], &["001","测试甲"], &["002","测试乙"]]);
        let preview = student_preview(&db, 1, valid, false).unwrap();
        assert!(preview.errors.is_empty());
        save_students(&mut db,1,preview.students,preview.snapshot,false).unwrap();
        let exam = Exam { id:0,class_id:1,name:"考试".into(),exam_date:"".into(),remark:String::new(),rank_grading:false,is_current:true,thresholds:[90,80,70,60],
            subjects:vec![Subject{id:0,name:"语文".into(),full_score:10000,counts_toward_total:true}] };
        let exam_id = scores::save_exam(&mut db,exam,None).unwrap();
        let bad = workbook(&[&["学号","语文"], &["001","90"], &["999","50"]]);
        let preview = score_preview(&db,exam_id,bad).unwrap();
        assert_eq!(preview.errors.len(),1);
        assert!(scores::score_rows(&db,exam_id).unwrap()[0].details.is_empty());
        let good = workbook(&[&["学号","语文"], &["001","90.5"], &["002","缺考"]]);
        let preview = score_preview(&db,exam_id,good).unwrap();
        assert!(preview.errors.is_empty());
        scores::save_scores(&mut db,exam_id,preview.changes,Some(preview.snapshot)).unwrap();
        let rows = scores::score_rows(&db,exam_id).unwrap();
        assert_eq!(rows[0].total_score,Some(9050));
        assert_eq!(rows[1].total_score,None);
        assert_eq!(rows[1].details[0].status,"absent");
        assert!(!score_template(&db,exam_id).unwrap().is_empty());
        assert!(!student_template().unwrap().is_empty());
    }
}
