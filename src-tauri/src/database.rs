use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const MIGRATION: &str = include_str!("../migrations/001_init.sql");
const SCORE_MIGRATION: &str = include_str!("../migrations/002_scores.sql");
const PLAN_MIGRATION: &str = include_str!("../migrations/003_plans.sql");
const SOURCE_MIGRATION: &str = include_str!("../migrations/004_plan_source.sql");
const PARITY_MIGRATION: &str = include_str!("../migrations/005_business_parity.sql");
const MIGRATIONS: [&str; 5] = [MIGRATION, SCORE_MIGRATION, PLAN_MIGRATION, SOURCE_MIGRATION, PARITY_MIGRATION];
pub const VERSION: i64 = MIGRATIONS.len() as i64;

pub fn schema_at(version: i64) -> rusqlite::Result<Connection> {
    let connection = Connection::open_in_memory()?;
    for migration in &MIGRATIONS[..version as usize] {
        connection.execute_batch(migration)?;
    }
    Ok(connection)
}

pub fn initialize(connection: &mut Connection) -> Result<(), String> {
    connection.pragma_update(None, "foreign_keys", true).map_err(error)?;
    connection.busy_timeout(std::time::Duration::from_secs(5)).map_err(error)?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).map_err(error)?;
    if version > VERSION {
        return Err("数据库版本高于当前应用版本，请使用较新版本打开".into());
    }
    if version < VERSION {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let current: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0)).map_err(error)?;
        if current > VERSION {
            return Err("数据库版本高于当前应用版本，请使用较新版本打开".into());
        }
        if current < VERSION {
            let index = usize::try_from(current).map_err(|_| "数据库版本无效")?;
            for migration in &MIGRATIONS[index..] {
                transaction.execute_batch(migration).map_err(error)?;
            }
            transaction.pragma_update(None, "user_version", VERSION).map_err(error)?;
        }
        transaction.commit().map_err(error)?;
    }
    Ok(())
}

fn error(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    if message.contains("UNIQUE constraint failed: seat_student.class_id, seat_student.student_no") {
        "该班级已有相同学号的在用学生".into()
    } else if message.contains("UNIQUE constraint failed: seat_position.classroom_id, seat_position.row_number, seat_position.column_number") {
        "该教室已有相同位置的座位".into()
    } else if message.contains("UNIQUE constraint failed: seat_student_relation.") {
        "该学生关系已存在，请编辑原有记录".into()
    } else if message.contains("FOREIGN KEY constraint failed") {
        "关联的数据不存在或不属于当前班级".into()
    } else {
        message
    }
}

fn fields(resource: &str) -> Result<(&'static str, &'static [&'static str]), String> {
    match resource {
        "classes" => Ok(("seat_class", &["name", "school_stage", "grade", "school_year", "term", "subjects", "active", "remark"])),
        "students" => Ok(("seat_student", &["class_id", "student_no", "name", "gender", "height", "eyesight", "score_level", "discipline", "notes", "sort_no", "active", "remark"])),
        "classrooms" => Ok(("seat_classroom", &["class_id", "name", "rows", "columns", "podium", "is_default", "active", "remark"])),
        "positions" => Ok(("seat_position", &["class_id", "classroom_id", "row_number", "column_number", "kind", "desk_group", "active"])),
        "rules" => Ok(("seat_rule", &["class_id", "name", "kind", "weight", "config", "enabled", "active", "remark"])),
        "relations" => Ok(("seat_student_relation", &["class_id", "student_id", "other_student_id", "kind", "active"])),
        _ => Err("未知数据类型".into()),
    }
}

fn sql_value(value: &Value) -> Result<SqlValue, String> {
    match value {
        Value::Null => Ok(SqlValue::Null),
        Value::String(value) => Ok(SqlValue::Text(value.trim().to_string())),
        Value::Number(value) if value.is_i64() => Ok(SqlValue::Integer(value.as_i64().unwrap())),
        Value::Number(value) => value.as_f64().map(SqlValue::Real).ok_or_else(|| "无效数字".into()),
        _ => Err("字段类型无效".into()),
    }
}

pub fn list(connection: &Connection, resource: &str, class_id: Option<i64>) -> Result<Vec<Value>, String> {
    let (table, columns) = fields(resource)?;
    let sql = if resource == "classes" {
        format!("SELECT id, {} FROM {table} ORDER BY id", columns.join(", "))
    } else {
        let order = if resource == "students" { "sort_no, student_no, id" } else { "id" };
        format!("SELECT id, {} FROM {table} WHERE class_id = ?1 ORDER BY {order}", columns.join(", "))
    };
    let mut statement = connection.prepare(&sql).map_err(error)?;
    let names: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
    let params: Vec<i64> = if resource == "classes" { vec![] } else { vec![class_id.ok_or("请选择班级")?] };
    let rows = statement.query_map(params_from_iter(params), |row| {
        let mut object = Map::new();
        for (index, name) in names.iter().enumerate() {
            let value: SqlValue = row.get(index)?;
            let value = match value {
                SqlValue::Null => Value::Null,
                SqlValue::Integer(number) => Value::from(number),
                SqlValue::Real(number) => Value::from(number),
                SqlValue::Text(text) => Value::from(text),
                SqlValue::Blob(_) => Value::Null,
            };
            object.insert(name.clone(), value);
        }
        Ok(Value::Object(object))
    }).map_err(error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(error)
}

fn field_label(name: &str) -> &str {
    match name {
        "id" => "记录编号",
        "class_id" => "班级",
        "classroom_id" => "教室",
        "student_id" => "学生",
        "other_student_id" => "另一位学生",
        "name" => "名称",
        "student_no" => "学号",
        "school_stage" => "学段",
        "grade" => "年级",
        "school_year" => "学年",
        "term" => "学期",
        "subjects" => "班级科目",
        "rows" => "座位行数",
        "columns" => "座位列数",
        "podium" => "讲台位置",
        "is_default" => "默认布局",
        "row_number" => "行号",
        "column_number" => "列号",
        "kind" => "类型",
        "desk_group" => "同桌分组",
        "weight" => "规则权重",
        "config" => "规则配置",
        "enabled" => "启用状态",
        "active" => "状态",
        "gender" => "性别",
        "height" => "身高",
        "eyesight" => "视力等级",
        "score_level" => "成绩等级",
        "discipline" => "纪律等级",
        "notes" => "特殊需求",
        "sort_no" => "排序号",
        "remark" => "备注",
        _ => name,
    }
}

fn required_integer(data: &Map<String, Value>, name: &str) -> Result<i64, String> {
    data.get(name).and_then(Value::as_i64).ok_or_else(|| match name {
        "class_id" | "classroom_id" | "student_id" | "other_student_id" => {
            format!("请选择{}", field_label(name))
        }
        _ => format!("{}必须是整数", field_label(name)),
    })
}

pub fn rule_students(config: &Value) -> Option<Vec<i64>> {
    let ids = match config.get("student_ids") {
        Some(array) => array.as_array()?.iter().map(Value::as_i64).collect::<Option<Vec<_>>>()?,
        None => vec![config.get("student_id")?.as_i64()?],
    };
    let unique: HashSet<_> = ids.iter().copied().collect();
    (!ids.is_empty() && ids.iter().all(|&id| id > 0) && unique.len() == ids.len()).then_some(ids)
}

fn same_record(current: &Map<String, Value>, expected: &Map<String, Value>) -> bool {
    current.len() == expected.len() && current.iter().all(|(name, value)| {
        expected.get(name).is_some_and(|other| match (value, other) {
            (Value::Number(left), Value::Number(right)) => {
                match (left.as_i64(), right.as_i64()) {
                    (Some(left), Some(right)) => left == right,
                    _ => left.as_f64().zip(right.as_f64()).is_some_and(|(left, right)| left == right),
                }
            }
            _ => value == other,
        })
    })
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PositionSnapshot {
    pub id: i64,
    pub kind: String,
    pub desk_group: String,
    pub active: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PositionChange {
    pub row_number: i64,
    pub column_number: i64,
    pub expected: Option<PositionSnapshot>,
    pub kind: String,
    pub desk_group: String,
    pub active: i64,
}

pub fn save_layout(
    connection: &mut Connection,
    class_id: i64,
    classroom_id: i64,
    expected_rows: i64,
    expected_columns: i64,
    changes: &[PositionChange],
) -> Result<(), String> {
    let transaction = connection.transaction().map_err(error)?;
    apply_layout(&transaction, class_id, classroom_id, expected_rows, expected_columns, changes)?;
    transaction.commit().map_err(error)
}

fn apply_layout(
    connection: &Connection, class_id: i64, classroom_id: i64, expected_rows: i64, expected_columns: i64,
    changes: &[PositionChange],
) -> Result<(), String> {
    let dimensions: Option<(i64, i64)> = connection
        .query_row(
            "SELECT rows, columns FROM seat_classroom WHERE id=?1 AND class_id=?2 AND active=1",
            params![classroom_id, class_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(error)?;
    if dimensions != Some((expected_rows, expected_columns)) {
        return Err("教室已变更或停用，请重新打开布局".into());
    }
    let mut coordinates = HashSet::new();
    for change in changes {
        if !(1..=expected_rows).contains(&change.row_number)
            || !(1..=expected_columns).contains(&change.column_number)
            || !coordinates.insert((change.row_number, change.column_number))
            || !["seat", "unavailable", "aisle"].contains(&change.kind.as_str())
            || ![0, 1].contains(&change.active)
            || change.desk_group.len() > 128
        {
            return Err("座位位置或设置无效".into());
        }
        let current = connection
            .query_row(
                "SELECT id, kind, desk_group, active FROM seat_position WHERE classroom_id=?1 AND row_number=?2 AND column_number=?3",
                params![classroom_id, change.row_number, change.column_number],
                |row| Ok(PositionSnapshot { id: row.get(0)?, kind: row.get(1)?, desk_group: row.get(2)?, active: row.get(3)? }),
            )
            .optional()
            .map_err(error)?;
        if current != change.expected {
            return Err("座位布局已发生变化，请重新打开布局后再保存".into());
        }
    }
    for change in changes {
        if let Some(expected) = &change.expected {
            connection.execute(
                "UPDATE seat_position SET kind=?1, desk_group=?2, active=?3 WHERE id=?4 AND classroom_id=?5 AND class_id=?6",
                params![change.kind, change.desk_group.trim(), change.active, expected.id, classroom_id, class_id],
            ).map_err(error)?;
        } else {
            connection.execute(
                "INSERT INTO seat_position(class_id, classroom_id, row_number, column_number, kind, desk_group, active) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![class_id, classroom_id, change.row_number, change.column_number, change.kind, change.desk_group.trim(), change.active],
            ).map_err(error)?;
        }
    }
    Ok(())
}

pub fn save_checked(connection: &mut Connection, resource: &str, data: Map<String, Value>, expected: Option<Map<String, Value>>) -> Result<i64, String> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
    let id = save_record_in_transaction(&transaction, resource, data, expected)?;
    transaction.commit().map_err(error)?;
    Ok(id)
}

pub fn deactivate_records(connection: &mut Connection, resource: &str, class_id: Option<i64>, expected: Vec<Map<String, Value>>) -> Result<(), String> {
    let (table, _) = fields(resource)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
    let current: HashMap<_, _> = list(&transaction, resource, class_id)?.into_iter()
        .filter_map(|record| Some((record["id"].as_i64()?, record.as_object()?.clone()))).collect();
    let mut ids = HashSet::new();
    for record in &expected {
        let id = required_integer(record, "id")?;
        if !ids.insert(id) || !current.get(&id).is_some_and(|item| same_record(item, record)) {
            return Err("资料已发生变化，请重新加载并选择记录".into());
        }
    }
    for id in ids {
        transaction.execute(&format!("UPDATE {table} SET active=0 WHERE id=?1"), [id]).map_err(error)?;
    }
    transaction.commit().map_err(error)
}

pub fn save_classroom_with_layout(connection: &mut Connection, data: Map<String, Value>, expected: Option<Map<String, Value>>, changes: &[PositionChange]) -> Result<i64, String> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
    let class_id = required_integer(&data, "class_id")?;
    let rows = required_integer(&data, "rows")?;
    let columns = required_integer(&data, "columns")?;
    let id = save_record_in_transaction(&transaction, "classrooms", data, expected)?;
    if !changes.is_empty() { apply_layout(&transaction, class_id, id, rows, columns, changes)?; }
    transaction.commit().map_err(error)?;
    Ok(id)
}

fn save_record_in_transaction(transaction: &Connection, resource: &str, data: Map<String, Value>, expected: Option<Map<String, Value>>) -> Result<i64, String> {
    match (data.get("id").and_then(Value::as_i64), expected) {
        (Some(id), Some(expected)) => {
            if expected.get("id").and_then(Value::as_i64) != Some(id) {
                return Err("原记录不匹配，请重新打开后编辑".into());
            }
            let class_id = if resource == "classes" { None } else { Some(required_integer(&data, "class_id")?) };
            let current = list(transaction, resource, class_id)?.into_iter().find(|item| item.get("id").and_then(Value::as_i64) == Some(id));
            if !current.as_ref().and_then(Value::as_object).is_some_and(|current| same_record(current, &expected)) {
                return Err("记录已发生变化，请重新打开后编辑".into());
            }
        }
        (None, None) => {}
        _ => return Err("缺少原记录信息，请重新打开后编辑".into()),
    }
    if resource == "classrooms" && data.get("is_default").and_then(Value::as_i64) == Some(1)
        && data.get("active").and_then(Value::as_i64) == Some(1) {
        let class_id = required_integer(&data, "class_id")?;
        let except = data.get("id").and_then(Value::as_i64).unwrap_or(0);
        transaction.execute("UPDATE seat_classroom SET is_default=0 WHERE class_id=? AND id<>? AND is_default=1 AND active=1",
            params![class_id, except]).map_err(error)?;
    }
    save(transaction, resource, data)
}

pub fn save(connection: &Connection, resource: &str, mut data: Map<String, Value>) -> Result<i64, String> {
    let (table, columns) = fields(resource)?;
    for (field, label) in match resource {
        "classes" => &[("name", "班级名称")][..],
        "students" => &[("student_no", "学号"), ("name", "学生姓名")][..],
        "classrooms" => &[("name", "教室名称")][..],
        "rules" => &[("name", "规则名称")][..],
        _ => &[][..],
    } {
        if data.get(*field).and_then(Value::as_str).map(str::trim).unwrap_or("").is_empty() {
            return Err(format!("{label}不能为空"));
        }
    }
    for field in match resource {
        "classrooms" => &["rows", "columns"][..],
        "rules" => &["weight"][..],
        _ => &[][..],
    } {
        required_integer(&data, field)?;
    }
    if resource == "classrooms" {
        if !(1..=50).contains(&required_integer(&data, "rows")?) || !(1..=50).contains(&required_integer(&data, "columns")?) {
            return Err("教室行数和列数须为 1 至 50 的整数".into());
        }
    }
    if resource == "rules" && !(0..=1000).contains(&required_integer(&data, "weight")?) {
        return Err("规则权重须为 0 至 1000 的整数".into());
    }
    if resource == "students" {
        if let Some(height) = data.get("height").filter(|value| !value.is_null()) {
            if height.as_f64().filter(|value| *value > 0.0).is_none() {
                return Err("身高须为空或大于 0".into());
            }
        }
    }
    if resource == "relations" {
        let first = required_integer(&data, "student_id")?;
        let second = required_integer(&data, "other_student_id")?;
        if first == second { return Err("不能选择同一个学生".into()); }
        data.insert("student_id".into(), Value::from(first.min(second)));
        data.insert("other_student_id".into(), Value::from(first.max(second)));
    }
    if resource == "rules" {
        let config = data.get("config").and_then(Value::as_str).ok_or("规则配置必须是 JSON")?;
        let parsed: Value = serde_json::from_str(config).map_err(|_| "规则配置不是有效的 JSON")?;
        if !parsed.is_object() { return Err("规则配置必须是 JSON 对象".into()); }
        let disabling_unchanged = match (data.get("id").and_then(Value::as_i64), data.get("active").and_then(Value::as_i64)) {
            (Some(id), Some(0)) => {
                let class_id = required_integer(&data, "class_id")?;
                let kind = data.get("kind").and_then(Value::as_str).ok_or("规则类型无效")?;
                connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM seat_rule WHERE id=?1 AND class_id=?2 AND config=?3 AND kind=?4)",
                    params![id, class_id, config, kind], |row| row.get::<_, bool>(0),
                ).map_err(error)?
            }
            _ => false,
        };
        let rule_type = parsed.get("type").and_then(Value::as_str);
        let expected_kind = match rule_type {
            Some("FRONT_ROW" | "NOT_DESKMATE" | "NOT_ADJACENT") => Some("HARD"),
            Some("PREFER_DESKMATE" | "VISION_FRONT" | "HEIGHT_BACK" | "GENDER_BALANCE" | "SCORE_BALANCE" | "DISCIPLINE_SCATTER") => Some("SOFT"),
            _ => None,
        };
        if let Some(expected) = expected_kind.filter(|_| !disabling_unchanged) {
            if data.get("kind").and_then(Value::as_str) != Some(expected) {
                return Err("规则类型与硬软约束不匹配".into());
            }
        }
        if matches!(rule_type, Some("FRONT_ROW" | "NOT_DESKMATE" | "NOT_ADJACENT" | "PREFER_DESKMATE")) && !disabling_unchanged {
            let ids = rule_students(&parsed).ok_or("规则请选择有效学生")?;
            if rule_type == Some("FRONT_ROW") {
                let max_row = parsed.get("max_row").and_then(Value::as_i64).ok_or("前排规则的排数必须是整数")?;
                if !(1..=50).contains(&max_row) { return Err("前排规则的排数应为 1 至 50".into()); }
            } else if ids.len() < 2 {
                return Err("学生关系规则至少选择两名学生".into());
            }
            let class_id = required_integer(&data, "class_id")?;
            for student_id in ids {
                let exists: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM seat_student WHERE id=?1 AND class_id=?2)",
                    params![student_id, class_id], |row| row.get(0),
                ).map_err(error)?;
                if !exists { return Err("规则的学生不存在或不属于当前班级".into()); }
            }
        }
    }
    if resource == "positions" {
        let class_id = required_integer(&data, "class_id")?;
        let classroom_id = required_integer(&data, "classroom_id")?;
        let row = required_integer(&data, "row_number")?;
        let column = required_integer(&data, "column_number")?;
        let valid: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM seat_classroom WHERE id=?1 AND class_id=?2 AND active=1 AND ?3 BETWEEN 1 AND rows AND ?4 BETWEEN 1 AND columns)",
            params![classroom_id, class_id, row, column], |row| row.get(0)
        ).map_err(error)?;
        if !valid { return Err("座位超出教室范围或教室不可用".into()); }
    }
    if resource == "classrooms" && data.get("id").and_then(Value::as_i64).is_some() {
        let id = required_integer(&data, "id")?;
        let rows = required_integer(&data, "rows")?;
        let columns_count = required_integer(&data, "columns")?;
        let out_of_bounds: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM seat_position WHERE classroom_id=?1 AND active=1 AND (row_number>?2 OR column_number>?3))",
            params![id, rows, columns_count], |row| row.get(0)
        ).map_err(error)?;
        if out_of_bounds { return Err("仍有启用的位置超出新范围，请先在布局中停用这些位置".into()); }
    }
    let mut values = Vec::new();
    let columns: Vec<_> = columns.iter().copied().filter(|name| {
        data.contains_key(*name) || !matches!(*name, "school_year" | "remark" | "score_level" | "sort_no" | "is_default" | "enabled")
    }).collect();
    for &name in &columns {
        let value = data.get(name).ok_or_else(|| format!("缺少{}", field_label(name)))?;
        values.push(if resource == "rules" && name == "config" {
            SqlValue::Text(value.as_str().ok_or("规则配置必须是 JSON")?.to_string())
        } else {
            sql_value(value)?
        });
    }
    if let Some(id) = data.get("id").and_then(Value::as_i64) {
        if resource != "classes" {
            let class_id = required_integer(&data, "class_id")?;
            let previous: Option<i64> = connection.query_row(&format!("SELECT class_id FROM {table} WHERE id=?1"), [id], |row| row.get(0)).optional().map_err(error)?;
            if previous != Some(class_id) { return Err("记录不存在或不属于当前班级".into()); }
        }
        let assignments = columns.iter().map(|name| format!("{name}=?")).collect::<Vec<_>>().join(", ");
        values.push(SqlValue::Integer(id));
        let changed = connection.execute(&format!("UPDATE {table} SET {assignments} WHERE id=?"), params_from_iter(values)).map_err(error)?;
        if changed != 1 { return Err("记录不存在".into()); }
        Ok(id)
    } else {
        let placeholders = vec!["?"; columns.len()].join(", ");
        connection.execute(&format!("INSERT INTO {table} ({}) VALUES ({placeholders})", columns.join(", ")), params_from_iter(values)).map_err(error)?;
        Ok(connection.last_insert_rowid())
    }
}

#[derive(Deserialize)]
pub struct CopyOptions {
    pub source_id: i64,
    pub name: String,
    pub term: String,
    #[serde(default)]
    pub school_year: Option<String>,
    pub students: bool,
    pub relations: bool,
    pub rules: bool,
    pub classrooms: bool,
}

pub fn copy_class(connection: &mut Connection, options: CopyOptions) -> Result<i64, String> {
    let CopyOptions { source_id, name, term, school_year, students, relations, rules, classrooms } = options;
    if name.trim().is_empty() || term.trim().is_empty() { return Err("新班级名称和学期不能为空".into()); }
    if relations && !students { return Err("复制学生关系时必须同时复制学生".into()); }
    let transaction = connection.transaction().map_err(error)?;
    let changed = transaction.execute(
        "INSERT INTO seat_class(name, school_stage, grade, term, subjects, active, school_year, remark)
         SELECT ?1, school_stage, grade, ?2, subjects, 1, coalesce(?3, school_year), remark FROM seat_class WHERE id=?4",
        params![name.trim(), term.trim(), school_year.as_deref().map(str::trim), source_id]
    ).map_err(error)?;
    if changed != 1 { return Err("原班级不存在".into()); }
    let target_id = transaction.last_insert_rowid();
    let mut mapping = HashMap::new();
    if students {
        let mut statement = transaction.prepare("SELECT id FROM seat_student WHERE class_id=?1 AND active=1 ORDER BY id").map_err(error)?;
        let ids = statement.query_map([source_id], |row| row.get::<_, i64>(0)).map_err(error)?.collect::<Result<Vec<_>, _>>().map_err(error)?;
        drop(statement);
        for id in ids {
            transaction.execute("INSERT INTO seat_student(class_id, student_no, name, gender, height, eyesight, discipline, notes, active, sort_no, remark, score_level) SELECT ?1, student_no, name, gender, height, eyesight, discipline, notes, 1, sort_no, remark, score_level FROM seat_student WHERE id=?2", params![target_id, id]).map_err(error)?;
            mapping.insert(id, transaction.last_insert_rowid());
        }
    }
    if relations {
        let mut statement = transaction.prepare("SELECT student_id, other_student_id, kind FROM seat_student_relation WHERE class_id=?1 AND active=1").map_err(error)?;
        let items = statement.query_map([source_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))).map_err(error)?.collect::<Result<Vec<_>, _>>().map_err(error)?;
        drop(statement);
        for (first, second, kind) in items {
            if let (Some(first), Some(second)) = (mapping.get(&first), mapping.get(&second)) {
                transaction.execute("INSERT INTO seat_student_relation(class_id, student_id, other_student_id, kind, active) VALUES(?1, ?2, ?3, ?4, 1)", params![target_id, first.min(second), first.max(second), kind]).map_err(error)?;
            }
        }
    }
    if rules {
        let mut statement = transaction.prepare("SELECT name, kind, weight, config, enabled, remark FROM seat_rule WHERE class_id=?1 AND active=1 ORDER BY id").map_err(error)?;
        let items = statement.query_map([source_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, String>(5)?))).map_err(error)?.collect::<Result<Vec<_>, _>>().map_err(error)?;
        drop(statement);
        for (name, kind, weight, config, enabled, remark) in items {
            let mut parsed: Value = serde_json::from_str(&config).map_err(|_| "原班级规则配置无效，无法复制")?;
            let target_config = if matches!(parsed.get("type").and_then(Value::as_str), Some("FRONT_ROW" | "NOT_DESKMATE" | "NOT_ADJACENT" | "PREFER_DESKMATE")) {
                let ids = rule_students(&parsed).ok_or("原班级规则缺少学生，无法复制")?;
                let mapped = ids.iter().map(|id| mapping.get(id).copied().ok_or("复制指定学生规则时须同时复制对应的在用学生"))
                    .collect::<Result<Vec<_>, _>>()?;
                if parsed.get("student_ids").is_some() { parsed["student_ids"] = Value::from(mapped); }
                else { parsed["student_id"] = Value::from(mapped[0]); }
                serde_json::to_string(&parsed).map_err(error)?
            } else { config };
            transaction.execute("INSERT INTO seat_rule(class_id, name, kind, weight, config, active, enabled, remark) VALUES(?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)", params![target_id, name, kind, weight, target_config, enabled, remark]).map_err(error)?;
        }
    }
    if classrooms {
        let mut statement = transaction.prepare(
            "SELECT id FROM seat_classroom WHERE class_id=?1 AND active=1
             AND (is_default=1 OR NOT EXISTS(SELECT 1 FROM seat_classroom WHERE class_id=?1 AND active=1 AND is_default=1))
             ORDER BY id").map_err(error)?;
        let ids = statement.query_map([source_id], |row| row.get::<_, i64>(0)).map_err(error)?.collect::<Result<Vec<_>, _>>().map_err(error)?;
        drop(statement);
        for id in ids {
            transaction.execute("INSERT INTO seat_classroom(class_id, name, rows, columns, podium, active, is_default, remark) SELECT ?1, name, rows, columns, podium, 1, is_default, remark FROM seat_classroom WHERE id=?2", params![target_id, id]).map_err(error)?;
            let new_id = transaction.last_insert_rowid();
            transaction.execute("INSERT INTO seat_position(class_id, classroom_id, row_number, column_number, kind, desk_group, active) SELECT ?1, ?2, row_number, column_number, kind, desk_group, active FROM seat_position WHERE classroom_id=?3", params![target_id, new_id, id]).map_err(error)?;
        }
    }
    transaction.commit().map_err(error)?;
    Ok(target_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parity_migration_preserves_existing_layouts_scores_and_rules() {
        let mut db = schema_at(4).unwrap();
        db.execute_batch("PRAGMA user_version=4;
            INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(7,'测试班','小学','六年级','旧学期');
            INSERT INTO seat_student(id,class_id,student_no,name,eyesight) VALUES(10,7,'001','测试学生','近视');
            INSERT INTO seat_classroom(id,class_id,name,rows,columns,podium) VALUES(8,7,'测试教室',2,2,'back');
            INSERT INTO seat_position(id,class_id,classroom_id,row_number,column_number,kind,desk_group) VALUES(20,7,8,1,1,'seat','原分组');
            INSERT INTO seat_rule(id,class_id,name,kind,weight,config) VALUES(33,7,'规则','SOFT',80,'{}');
            INSERT INTO seat_exam(id,class_id,name) VALUES(28,7,'旧考试');
            INSERT INTO seat_student_score(exam_id,student_id,class_id,total_score,total_rank,total_grade) VALUES(28,10,7,7900,1,'C');").unwrap();
        initialize(&mut db).unwrap();
        assert_eq!(list(&db, "classes", None).unwrap()[0]["term"], "旧学期");
        assert_eq!(list(&db, "positions", Some(7)).unwrap()[0]["desk_group"], "原分组");
        assert_eq!(list(&db, "rules", Some(7)).unwrap()[0]["enabled"], 1);
        let old_exam = &crate::scores::exams(&db, 7).unwrap()[0];
        assert!(!old_exam.rank_grading);
        assert_eq!(crate::scores::score_rows(&db, 28).unwrap()[0].total_grade.as_deref(), Some("C"));
        assert!(!db.prepare("PRAGMA foreign_key_check").unwrap().exists([]).unwrap());
        db.execute("UPDATE seat_classroom SET podium='left' WHERE id=8", []).unwrap();
    }

    fn copy_class(connection: &mut Connection, source_id: i64, name: &str, term: &str, students: bool, relations: bool, rules: bool, classrooms: bool) -> Result<i64, String> {
        super::copy_class(connection, CopyOptions {
            source_id, name: name.into(), term: term.into(), school_year: None, students, relations, rules, classrooms,
        })
    }

    fn data(value: Value) -> Map<String, Value> { value.as_object().unwrap().clone() }
    fn class(connection: &Connection, name: &str) -> i64 {
        save(connection, "classes", data(json!({"name":name,"school_stage":"小学","grade":"一年级","term":"上学期","subjects":"语文","active":1}))).unwrap()
    }
    fn student(connection: &Connection, class_id: i64, number: &str) -> i64 {
        save(connection, "students", data(json!({"class_id":class_id,"student_no":number,"name":"测试学生","gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":1}))).unwrap()
    }

    #[test]
    fn plan_migration_preserves_version_two_data_and_rolls_back() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATION).unwrap();
        connection.execute_batch(SCORE_MIGRATION).unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        connection.execute_batch("INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(1,'测试班','','','');
            INSERT INTO seat_exam(class_id,name) VALUES(1,'历史考试');
            CREATE TABLE seat_plan_score(id INTEGER);").unwrap();
        assert!(initialize(&mut connection).is_err());
        let version = |db: &Connection| db.pragma_query_value(None,"user_version",|r| r.get::<_,i64>(0)).unwrap();
        assert_eq!(version(&connection),2);
        assert!(connection.prepare("SELECT * FROM seat_plan").is_err());
        connection.execute_batch("DROP TABLE seat_plan_score").unwrap();
        initialize(&mut connection).unwrap();
        assert_eq!(version(&connection),VERSION);
        assert_eq!(connection.query_row("SELECT name FROM seat_exam",[],|r| r.get::<_,String>(0)).unwrap(),"历史考试");
    }

    #[test]
    fn score_migration_upgrades_v1_and_rolls_back_on_failure() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATION).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection.execute("INSERT INTO seat_class(name, school_stage, grade, term) VALUES('测试班', '初中', '七年级', '上学期')", []).unwrap();
        connection.execute("CREATE TABLE seat_exam(id INTEGER)", []).unwrap();
        assert!(initialize(&mut connection).is_err());
        assert_eq!(connection.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='seat_subject'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM seat_class", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        connection.execute("DROP TABLE seat_exam", []).unwrap();
        initialize(&mut connection).unwrap();
        assert_eq!(connection.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(), VERSION);
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM seat_class", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn score_schema_rejects_cross_class_and_multiple_current_exams() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        connection.execute("INSERT INTO seat_class(id, name, school_stage, grade, term) VALUES(1, '一班', '', '', ''), (2, '二班', '', '', '')", []).unwrap();
        connection.execute("INSERT INTO seat_student(id, class_id, student_no, name) VALUES(1, 1, '001', '测试甲'), (2, 2, '002', '测试乙')", []).unwrap();
        connection.execute("INSERT INTO seat_exam(id, class_id, name, is_current) VALUES(1, 1, '考试一', 1), (2, 2, '考试二', 1)", []).unwrap();
        assert!(connection.execute("INSERT INTO seat_exam(class_id, name, is_current) VALUES(1, '考试三', 1)", []).is_err());
        assert!(connection.execute("INSERT INTO seat_student_score(exam_id, student_id, class_id) VALUES(1, 2, 1)", []).is_err());
        connection.execute("INSERT INTO seat_student_score(exam_id, student_id, class_id) VALUES(1, 1, 1)", []).unwrap();
        connection.execute("INSERT INTO seat_exam_subject(id, exam_id, name, full_score) VALUES(1, 2, '语文', 10000)", []).unwrap();
        assert!(connection.execute("INSERT INTO seat_student_score_detail(exam_id, student_id, exam_subject_id, status, score) VALUES(1, 1, 1, 'scored', 9000)", []).is_err());
    }

    #[test]
    fn migration_is_atomic_and_rejects_newer_versions() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute("CREATE TABLE seat_student(id INTEGER)", []).unwrap();
        connection.execute("INSERT INTO seat_student(id) VALUES(42)", []).unwrap();
        assert!(initialize(&mut connection).is_err());
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 0);
        let created_classes: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='seat_class'", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(created_classes, 0);
        assert_eq!(connection.query_row("SELECT id FROM seat_student", [], |row| row.get::<_, i64>(0)).unwrap(), 42);
        connection.pragma_update(None, "user_version", 99).unwrap();
        assert!(initialize(&mut connection).is_err());
    }

    #[test]
    fn constraints_and_copy_are_enforced() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let first = class(&connection, "一班");
        let second = class(&connection, "二班");
        let student_a = student(&connection, first, "001");
        let student_b = student(&connection, first, "002");
        let other = student(&connection, second, "001");
        assert!(save(&connection, "students", data(json!({"class_id":first,"student_no":"001","name":"重复","gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":1}))).is_err());
        let relation = |left, right| data(json!({"class_id":first,"student_id":left,"other_student_id":right,"kind":"NOT_DESKMATE","active":1}));
        assert!(save(&connection, "relations", relation(student_a, other)).is_err());
        save(&connection, "relations", relation(student_b, student_a)).unwrap();
        assert!(save(&connection, "relations", relation(student_a, student_b)).is_err());
        assert!(copy_class(&mut connection, first, "新班", "下学期", false, true, false, false).is_err());
        let copied = copy_class(&mut connection, first, "新班", "下学期", true, true, false, false).unwrap();
        assert_eq!(list(&connection, "students", Some(copied)).unwrap().len(), 2);
        assert_eq!(list(&connection, "relations", Some(copied)).unwrap().len(), 1);
        assert_eq!(list(&connection, "classes", None).unwrap().len(), 3);
    }

    #[test]
    fn data_survives_reopening() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("phase2-persistence-{}.db", std::process::id()));
        {
            let mut connection = Connection::open(&path).unwrap();
            initialize(&mut connection).unwrap();
            let class_id = class(&connection, "持久化测试");
            student(&connection, class_id, "001");
        }
        let mut connection = Connection::open(&path).unwrap();
        initialize(&mut connection).unwrap();
        let classes = list(&connection, "classes", None).unwrap();
        assert_eq!(classes.len(), 1);
        let class_id = classes[0]["id"].as_i64().unwrap();
        assert_eq!(list(&connection, "students", Some(class_id)).unwrap().len(), 1);
    }

    #[test]
    fn positions_stay_within_their_classroom_and_copy() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let first = class(&connection, "一班");
        let second = class(&connection, "二班");
        let classroom = save(&connection, "classrooms", data(json!({"class_id":first,"name":"教室","rows":2,"columns":2,"podium":"front","active":1}))).unwrap();
        let position = |class_id, row| data(json!({"class_id":class_id,"classroom_id":classroom,"row_number":row,"column_number":1,"kind":"seat","desk_group":"","active":1}));
        assert!(save(&connection, "positions", position(second, 1)).is_err());
        assert!(save(&connection, "positions", position(first, 3)).is_err());
        save(&connection, "positions", position(first, 1)).unwrap();
        let copied = copy_class(&mut connection, first, "新班", "下学期", false, false, false, true).unwrap();
        assert_eq!(list(&connection, "classrooms", Some(copied)).unwrap().len(), 1);
        assert_eq!(list(&connection, "positions", Some(copied)).unwrap().len(), 1);
    }

    #[test]
    fn rejects_fractional_integer_fields() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        let classroom = data(json!({"class_id":class_id,"name":"教室","rows":2.5,"columns":2,"podium":"front","active":1}));
        let rule = data(json!({"class_id":class_id,"name":"规则","kind":"SOFT","weight":1.5,"config":"{}","active":1}));
        assert!(save(&connection, "classrooms", classroom).is_err());
        assert!(save(&connection, "rules", rule).is_err());
    }

    #[test]
    fn duplicate_student_number_has_actionable_error() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        student(&connection, class_id, "001");
        let duplicate = data(json!({"class_id":class_id,"student_no":"001","name":"另一位学生","gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":1}));
        assert_eq!(save(&connection, "students", duplicate).unwrap_err(), "该班级已有相同学号的在用学生");
    }

    #[test]
    fn layout_saves_changes_atomically_and_rejects_stale_or_invalid_input() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        let other_class = class(&connection, "二班");
        let classroom_id = save(&connection, "classrooms", data(json!({"class_id":class_id,"name":"教室","rows":2,"columns":2,"podium":"front","active":1}))).unwrap();
        let first = PositionChange { row_number: 1, column_number: 1, expected: None, kind: "seat".into(), desk_group: "一组".into(), active: 1 };
        let second = PositionChange { row_number: 1, column_number: 2, expected: None, kind: "aisle".into(), desk_group: "".into(), active: 1 };
        assert!(save_layout(&mut connection, other_class, classroom_id, 2, 2, &[first.clone()]).is_err());
        assert!(save_layout(&mut connection, class_id, classroom_id, 2, 2, &[first.clone(), first.clone()]).is_err());
        assert!(save_layout(&mut connection, class_id, classroom_id, 2, 2, &[first.clone(), PositionChange { row_number: 3, ..second.clone() }]).is_err());
        assert!(list(&connection, "positions", Some(class_id)).unwrap().is_empty());
        save_layout(&mut connection, class_id, classroom_id, 2, 2, &[first.clone(), second]).unwrap();
        let before = list(&connection, "positions", Some(class_id)).unwrap();
        assert_eq!(before.len(), 2);
        assert!(save_layout(&mut connection, class_id, classroom_id, 2, 2, &[PositionChange { row_number: 2, ..first.clone() }, first.clone()]).is_err());
        assert_eq!(list(&connection, "positions", Some(class_id)).unwrap(), before);
        let position = &before[0];
        let updated = PositionChange {
            expected: Some(PositionSnapshot { id: position["id"].as_i64().unwrap(), kind: "seat".into(), desk_group: "一组".into(), active: 1 }),
            kind: "unavailable".into(),
            desk_group: "".into(),
            ..first
        };
        assert!(save_layout(&mut connection, class_id, classroom_id, 3, 2, &[updated.clone()]).is_err());
        save_layout(&mut connection, class_id, classroom_id, 2, 2, &[updated.clone()]).unwrap();
        assert!(save_layout(&mut connection, class_id, classroom_id, 2, 2, &[updated]).is_err());
        let copied = copy_class(&mut connection, class_id, "新班", "下学期", false, false, false, true).unwrap();
        let copied_positions = list(&connection, "positions", Some(copied)).unwrap();
        assert_eq!(copied_positions.len(), 2);
        assert!(copied_positions.iter().any(|position| position["kind"] == "unavailable"));
    }

    #[test]
    fn front_row_rule_validates_student_and_copies_with_new_student_id() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let first = class(&connection, "一班");
        let second = class(&connection, "二班");
        let first_student = student(&connection, first, "001");
        let other_student = student(&connection, second, "002");
        let rule = |student_id, max_row| data(json!({
            "class_id":first,"name":"指定前排","kind":"HARD","weight":1,
            "config":json!({"type":"FRONT_ROW","student_id":student_id,"max_row":max_row}).to_string(),
            "active":1
        }));
        assert!(save(&connection, "rules", rule(other_student, 2)).is_err());
        assert!(save(&connection, "rules", rule(first_student, 0)).is_err());
        save(&connection, "rules", rule(first_student, 2)).unwrap();
        assert!(copy_class(&mut connection, first, "新班", "下学期", false, false, true, false).is_err());
        assert_eq!(list(&connection, "classes", None).unwrap().len(), 2);
        let copied = copy_class(&mut connection, first, "新班", "下学期", true, false, true, false).unwrap();
        let new_student = list(&connection, "students", Some(copied)).unwrap()[0]["id"].as_i64().unwrap();
        let copied_rules = list(&connection, "rules", Some(copied)).unwrap();
        let config: Value = serde_json::from_str(copied_rules[0]["config"].as_str().unwrap()).unwrap();
        assert_ne!(first_student, new_student);
        assert_eq!(config["student_id"].as_i64(), Some(new_student));
        assert_eq!(config["max_row"].as_i64(), Some(2));
    }

    #[test]
    fn failed_copy_and_layout_writes_roll_back_every_change() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        student(&connection, class_id, "001");
        let classroom_id = save(&connection, "classrooms", data(json!({"class_id":class_id,"name":"教室","rows":2,"columns":2,"podium":"front","active":1}))).unwrap();
        connection.execute_batch("CREATE TRIGGER test_fail_position BEFORE INSERT ON seat_position WHEN NEW.column_number = 2 BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;").unwrap();
        let first = PositionChange { row_number: 1, column_number: 1, expected: None, kind: "seat".into(), desk_group: "".into(), active: 1 };
        let second = PositionChange { column_number: 2, ..first.clone() };
        assert!(save_layout(&mut connection, class_id, classroom_id, 2, 2, &[first, second]).is_err());
        assert!(list(&connection, "positions", Some(class_id)).unwrap().is_empty());

        save(&connection, "rules", data(json!({"class_id":class_id,"name":"规则","kind":"SOFT","weight":1,"config":"{}","active":1}))).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER test_fail_rule BEFORE INSERT ON seat_rule WHEN NEW.class_id <> {class_id} BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;")).unwrap();
        assert!(copy_class(&mut connection, class_id, "新班", "下学期", true, false, true, false).is_err());
        assert_eq!(list(&connection, "classes", None).unwrap().len(), 1);
        assert_eq!(list(&connection, "students", Some(class_id)).unwrap().len(), 1);
    }

    #[test]
    fn shrinking_classroom_preserves_inactive_outside_positions() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        let classroom_id = save(&connection, "classrooms", data(json!({"class_id":class_id,"name":"教室","rows":2,"columns":2,"podium":"front","active":1}))).unwrap();
        let position_id = save(&connection, "positions", data(json!({"class_id":class_id,"classroom_id":classroom_id,"row_number":2,"column_number":2,"kind":"seat","desk_group":"A","active":1}))).unwrap();
        let resize = |rows| data(json!({"id":classroom_id,"class_id":class_id,"name":"教室","rows":rows,"columns":2,"podium":"front","active":1}));
        assert!(save(&connection, "classrooms", resize(1)).is_err());
        save(&connection, "positions", data(json!({"id":position_id,"class_id":class_id,"classroom_id":classroom_id,"row_number":2,"column_number":2,"kind":"seat","desk_group":"A","active":0}))).unwrap();
        save(&connection, "classrooms", resize(1)).unwrap();
        let stored = list(&connection, "positions", Some(class_id)).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["desk_group"], "A");
        assert_eq!(stored[0]["active"], 0);
        save(&connection, "classrooms", resize(2)).unwrap();
        assert_eq!(list(&connection, "positions", Some(class_id)).unwrap(), stored);
    }

    #[test]
    fn required_fields_and_numeric_bounds_return_clear_errors() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let invalid_class = data(json!({"name":"  ","school_stage":"","grade":"","term":"","subjects":"","active":1}));
        assert_eq!(save(&connection, "classes", invalid_class).unwrap_err(), "班级名称不能为空");
        let class_id = class(&connection, "一班");
        let bad_student = data(json!({"class_id":class_id,"student_no":"","name":"测试学生","gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":1}));
        assert_eq!(save(&connection, "students", bad_student).unwrap_err(), "学号不能为空");
        let bad_height = data(json!({"class_id":class_id,"student_no":"001","name":"测试学生","gender":"","height":0,"eyesight":"","discipline":"","notes":"","active":1}));
        assert_eq!(save(&connection, "students", bad_height).unwrap_err(), "身高须为空或大于 0");
        let bad_classroom = data(json!({"class_id":class_id,"name":"教室","rows":51,"columns":1,"podium":"front","active":1}));
        assert_eq!(save(&connection, "classrooms", bad_classroom).unwrap_err(), "教室行数和列数须为 1 至 50 的整数");
    }

    #[test]
    fn copy_options_preserve_source_and_copy_only_selected_active_records() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let source = class(&connection, "一班");
        let first = student(&connection, source, "001");
        let second = student(&connection, source, "002");
        let inactive = student(&connection, source, "003");
        let relation = |left, right| data(json!({"class_id":source,"student_id":left,"other_student_id":right,"kind":"NOT_ADJACENT","active":1}));
        save(&connection, "relations", relation(first, second)).unwrap();
        save(&connection, "relations", relation(first, inactive)).unwrap();
        save(&connection, "students", data(json!({"id":inactive,"class_id":source,"student_no":"003","name":"测试学生","gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":0}))).unwrap();
        let original_students = list(&connection, "students", Some(source)).unwrap();
        let original_relations = list(&connection, "relations", Some(source)).unwrap();
        let empty = copy_class(&mut connection, source, "空班", "下学期", false, false, false, false).unwrap();
        assert!(list(&connection, "students", Some(empty)).unwrap().is_empty());
        assert!(list(&connection, "relations", Some(empty)).unwrap().is_empty());
        let copied = copy_class(&mut connection, source, "新班", "下学期", true, true, false, false).unwrap();
        let copied_students = list(&connection, "students", Some(copied)).unwrap();
        let copied_relations = list(&connection, "relations", Some(copied)).unwrap();
        assert_eq!(copied_students.len(), 2);
        assert_eq!(copied_relations.len(), 1);
        let student_ids: HashSet<i64> = copied_students.iter().map(|item| item["id"].as_i64().unwrap()).collect();
        assert!(student_ids.contains(&copied_relations[0]["student_id"].as_i64().unwrap()));
        assert!(student_ids.contains(&copied_relations[0]["other_student_id"].as_i64().unwrap()));
        assert_eq!(list(&connection, "students", Some(source)).unwrap(), original_students);
        assert_eq!(list(&connection, "relations", Some(source)).unwrap(), original_relations);
    }

    #[test]
    fn recognized_rules_cannot_use_the_wrong_constraint_kind() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        let rule = data(json!({"class_id":class_id,"name":"视力靠前","kind":"HARD","weight":1,"config":"{\"type\":\"VISION_FRONT\"}","active":1}));
        assert_eq!(save(&connection, "rules", rule).unwrap_err(), "规则类型与硬软约束不匹配");
        assert!(list(&connection, "rules", Some(class_id)).unwrap().is_empty());
    }

    #[test]
    fn legacy_invalid_rules_can_be_disabled_without_enabling_invalid_configs() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        let config = r#"  {"type":"FRONT_ROW"}  "#;
        connection.execute(
            "INSERT INTO seat_rule(class_id,name,kind,weight,config,active) VALUES(?1,'旧规则','HARD',1,?2,1)",
            params![class_id, config],
        ).unwrap();
        let original = list(&connection, "rules", Some(class_id)).unwrap()[0].as_object().unwrap().clone();
        let mut disabled = original.clone();
        disabled.insert("active".into(), json!(0));
        assert!(save_checked(&mut connection, "rules", disabled.clone(), Some(original)).is_ok());
        let stored = list(&connection, "rules", Some(class_id)).unwrap()[0].as_object().unwrap().clone();
        assert_eq!(stored["config"], config);
        assert_eq!(stored["active"], 0);
        let mut enabled = stored.clone();
        enabled.insert("active".into(), json!(1));
        assert!(save_checked(&mut connection, "rules", enabled, Some(stored.clone())).is_err());
        let mut altered = stored.clone();
        altered.insert("config".into(), json!("{\"type\":\"FRONT_ROW\",\"max_row\":2}"));
        assert!(save_checked(&mut connection, "rules", altered, Some(stored)).is_err());
        let new_invalid = data(json!({"class_id":class_id,"name":"新规则","kind":"HARD","weight":1,"config":config,"active":0}));
        assert!(save_checked(&mut connection, "rules", new_invalid, None).is_err());

        connection.execute(
            "INSERT INTO seat_rule(class_id,name,kind,weight,config,active) VALUES(?1,'旧软规则','HARD',1,?2,1)",
            params![class_id, r#"{"type":"VISION_FRONT"}"#],
        ).unwrap();
        let previous = list(&connection, "rules", Some(class_id)).unwrap()[1].as_object().unwrap().clone();
        let mut inactive = previous.clone();
        inactive.insert("active".into(), json!(0));
        assert!(save_checked(&mut connection, "rules", inactive, Some(previous)).is_ok());
    }

    #[test]
    fn checked_save_refuses_to_overwrite_concurrent_edits() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        let original = list(&connection, "classes", None).unwrap()[0].as_object().unwrap().clone();
        let mut first_edit = original.clone();
        first_edit.insert("name".into(), json!("首次修改"));
        assert_eq!(save_checked(&mut connection, "classes", first_edit.clone(), Some(original.clone())).unwrap(), class_id);
        let mut stale_edit = original.clone();
        stale_edit.insert("name".into(), json!("过期修改"));
        assert_eq!(save_checked(&mut connection, "classes", stale_edit.clone(), Some(original)).unwrap_err(), "记录已发生变化，请重新打开后编辑");
        assert!(save_checked(&mut connection, "classes", stale_edit, None).is_err());
        assert_eq!(list(&connection, "classes", None).unwrap()[0]["name"], "首次修改");
        let new_record = data(json!({"name":"二班","school_stage":"小学","grade":"一年级","term":"上学期","subjects":"","active":1}));
        assert!(save_checked(&mut connection, "classes", new_record, None).is_ok());
    }

    #[test]
    fn checked_save_accepts_integer_height_round_tripped_through_javascript() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize(&mut connection).unwrap();
        let class_id = class(&connection, "一班");
        let student_id = save(&connection, "students", data(json!({"class_id":class_id,"student_no":"001","name":"测试学生","gender":"","height":170.0,"eyesight":"","discipline":"","notes":"","active":1}))).unwrap();
        let mut expected = list(&connection, "students", Some(class_id)).unwrap()[0].as_object().unwrap().clone();
        expected.insert("height".into(), json!(170));
        let mut changed = expected.clone();
        changed.insert("name".into(), json!("修改姓名"));
        assert_eq!(save_checked(&mut connection, "students", changed, Some(expected)).unwrap(), student_id);
        assert_eq!(list(&connection, "students", Some(class_id)).unwrap()[0]["name"], "修改姓名");
    }

    #[test]
    fn every_copy_option_combination_preserves_references_and_atomicity() {
        for flags in 0..16 {
            let mut connection = Connection::open_in_memory().unwrap();
            initialize(&mut connection).unwrap();
            let source = class(&connection, "原班级");
            let first = student(&connection, source, "001");
            let second = student(&connection, source, "002");
            save(&connection, "relations", data(json!({
                "class_id":source,"student_id":first,"other_student_id":second,"kind":"NOT_DESKMATE","active":1
            }))).unwrap();
            save(&connection, "rules", data(json!({
                "class_id":source,"name":"前排","kind":"HARD","weight":1,
                "config":json!({"type":"FRONT_ROW","student_id":first,"max_row":2}).to_string(),"active":1
            }))).unwrap();
            let classroom = save(&connection, "classrooms", data(json!({
                "class_id":source,"name":"教室","rows":2,"columns":2,"podium":"front","active":1
            }))).unwrap();
            save(&connection, "positions", data(json!({
                "class_id":source,"classroom_id":classroom,"row_number":1,"column_number":1,
                "kind":"seat","desk_group":"第一组","active":1
            }))).unwrap();
            let before: Vec<Vec<Value>> = ["students", "relations", "rules", "classrooms", "positions"]
                .iter().map(|resource| list(&connection, resource, Some(source)).unwrap()).collect();
            let students = flags & 1 != 0;
            let relations = flags & 2 != 0;
            let rules = flags & 4 != 0;
            let classrooms = flags & 8 != 0;
            let result = copy_class(&mut connection, source, "新班级", "下学期", students, relations, rules, classrooms);
            if !students && (relations || rules) {
                assert!(result.is_err(), "flags={flags}");
                assert_eq!(list(&connection, "classes", None).unwrap().len(), 1, "flags={flags}");
            } else {
                let target = result.unwrap();
                let copied_students = list(&connection, "students", Some(target)).unwrap();
                let copied_relations = list(&connection, "relations", Some(target)).unwrap();
                let copied_rules = list(&connection, "rules", Some(target)).unwrap();
                let copied_classrooms = list(&connection, "classrooms", Some(target)).unwrap();
                let copied_positions = list(&connection, "positions", Some(target)).unwrap();
                assert_eq!(copied_students.len(), if students { 2 } else { 0 }, "flags={flags}");
                assert_eq!(copied_relations.len(), usize::from(relations), "flags={flags}");
                assert_eq!(copied_rules.len(), usize::from(rules), "flags={flags}");
                assert_eq!(copied_classrooms.len(), usize::from(classrooms), "flags={flags}");
                assert_eq!(copied_positions.len(), usize::from(classrooms), "flags={flags}");
                if relations {
                    assert_eq!(copied_relations[0]["student_id"], copied_students[0]["id"]);
                    assert_eq!(copied_relations[0]["other_student_id"], copied_students[1]["id"]);
                    assert_ne!(copied_students[0]["id"], first);
                }
                if rules {
                    let config: Value = serde_json::from_str(copied_rules[0]["config"].as_str().unwrap()).unwrap();
                    assert_eq!(config["student_id"], copied_students[0]["id"]);
                }
                if classrooms {
                    assert_eq!(copied_positions[0]["classroom_id"], copied_classrooms[0]["id"]);
                    assert_eq!(copied_positions[0]["desk_group"], "第一组");
                    assert_ne!(copied_classrooms[0]["id"], classroom);
                }
            }
            for (index, resource) in ["students", "relations", "rules", "classrooms", "positions"].iter().enumerate() {
                assert_eq!(list(&connection, resource, Some(source)).unwrap(), before[index], "flags={flags}");
            }
        }
    }

    #[test]
    fn concurrent_initialization_waits_for_committed_schema() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target")
            .join(format!("phase2-concurrent-{}-{stamp}.db", std::process::id()));
        let mut first = Connection::open(&path).unwrap();
        first.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        let transaction = first.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut second = Connection::open(path).unwrap();
            started.send(()).unwrap();
            initialize(&mut second)
        });
        ready.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        transaction.execute_batch(MIGRATION).unwrap();
        transaction.pragma_update(None, "user_version", 1).unwrap();
        transaction.commit().unwrap();
        assert!(worker.join().unwrap().is_ok());
        assert_eq!(first.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(), VERSION);
    }

    #[test]
    fn concurrent_initialization_never_downgrades_newer_schema() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target")
            .join(format!("phase2-future-{}-{stamp}.db", std::process::id()));
        let mut first = Connection::open(&path).unwrap();
        first.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        let transaction = first.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut second = Connection::open(path).unwrap();
            started.send(()).unwrap();
            initialize(&mut second)
        });
        ready.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        transaction.pragma_update(None, "user_version", VERSION + 1).unwrap();
        transaction.commit().unwrap();
        assert_eq!(worker.join().unwrap().unwrap_err(), "数据库版本高于当前应用版本，请使用较新版本打开");
        assert_eq!(first.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(), VERSION + 1);
        let tables: i64 = first.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table'", [], |row| row.get(0)).unwrap();
        assert_eq!(tables, 0);
    }
}
