use crate::{
    database, scores,
    seating::{
        self, Assignment, CellKind, FrontRule, PodiumSide, Position, Rating, Relation, RelationKind, RuleKind,
        Snapshot, SoftRule, Student,
    },
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Offline,
    Ai,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Ai => "ai",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: Option<i64>,
    pub name: String,
    pub confirmed: bool,
    #[serde(default)]
    pub archived: bool,
    pub revision: u32,
    pub seed: u32,
    #[serde(default)]
    pub source: Source,
    pub snapshot: Snapshot,
    pub assignments: Vec<Assignment>,
    pub ratings: Vec<Rating>,
}

#[derive(Serialize)]
pub struct PlanEntry {
    pub id: i64,
    pub name: String,
    pub confirmed: bool,
    pub archived: bool,
    pub source: Source,
    pub updated_at: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Adjustment {
    Move { from: i64, to: i64 },
    Lock { position: i64 },
    BatchLock { positions: Vec<i64>, locked: bool },
    Place { student_id: i64, position: i64 },
    Clear { position: i64 },
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn snapshot(
    db: &Connection,
    class_id: i64,
    classroom_id: i64,
    exam_id: Option<i64>,
) -> Result<Snapshot, String> {
    let class_name = db
        .query_row(
            "SELECT name FROM seat_class WHERE id=? AND active=1",
            [class_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?
        .ok_or("请选择在用班级")?;
    let (classroom_name, rows, columns, podium): (String,u16,u16,String) = db.query_row(
        "SELECT name,rows,columns,podium FROM seat_classroom WHERE id=? AND class_id=? AND active=1",
        params![classroom_id,class_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
        .optional().map_err(error)?.ok_or("请选择该班级的在用教室")?;
    let exam: Option<(String, bool)> = match exam_id {
        Some(id) => Some(
            db.query_row(
                "SELECT name,rank_grading FROM seat_exam WHERE id=? AND class_id=?",
                params![id, class_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(error)?
            .ok_or("考试不属于当前班级")?,
        ),
        None => None,
    };
    let use_score_levels = exam.as_ref().is_none_or(|(_, rank_grading)| *rank_grading);
    let ranks: std::collections::HashMap<_, _> = match exam_id.filter(|_| !use_score_levels) {
        Some(id) => scores::score_rows(db, id)?
            .into_iter()
            .filter(|s| s.active)
            .filter_map(|s| Some((s.student_id, u32::try_from(s.total_rank?).ok()?)))
            .collect(),
        None => Default::default(),
    };
    let records = database::list(db, "students", Some(class_id))?;
    let mut students = Vec::new();
    let mut unknown_vision = 0;
    let mut unknown_discipline = 0;
    for record in records.iter().filter(|s| s["active"] == 1) {
        let id = record["id"].as_i64().ok_or("学生编号无效")?;
        let vision = record["eyesight"].as_str().unwrap_or("").trim();
        let discipline = record["discipline"].as_str().unwrap_or("").trim();
        let poor_vision = matches!(vision, "1" | "2" | "3" | "轻度" | "中度" | "重度" | "较差" | "差" | "视力较差" | "近视" | "视力障碍");
        let discipline_attention = matches!(
            discipline,
            "1" | "2" | "关注" | "需关注" | "需要关注" | "重点关注" | "较差" | "差"
        );
        if !poor_vision && !matches!(vision, "0" | "正常" | "良好" | "好") {
            unknown_vision += 1;
        }
        if !discipline_attention && !matches!(discipline, "0" | "正常" | "良好" | "好") {
            unknown_discipline += 1;
        }
        students.push(Student {
            id,
            number: record["student_no"].as_str().unwrap_or("").into(),
            name: record["name"].as_str().unwrap_or("").into(),
            gender: match record["gender"].as_str() {
                Some("男") => Some(true),
                Some("女") => Some(false),
                _ => None,
            },
            height_tenths: record["height"]
                .as_f64()
                .and_then(|n| ((0.1..=1000.0).contains(&n)).then_some((n * 10.0).round() as u32)),
            poor_vision,
            discipline_attention,
            score_rank: ranks.get(&id).copied(),
            score_level: match record["score_level"].as_str() {
                Some("A") => Some(0), Some("B") => Some(1), Some("C") => Some(2),
                Some("D") => Some(3), Some("E") => Some(4), _ => None,
            },
        });
    }
    students.sort_by_key(|s| s.id);
    let mut stmt = db.prepare("SELECT id,row_number,column_number,kind,desk_group FROM seat_position WHERE classroom_id=? AND class_id=? AND active=1 AND row_number BETWEEN 1 AND ? AND column_number BETWEEN 1 AND ? ORDER BY row_number,column_number").map_err(error)?;
    let mut positions: Vec<Position> = stmt
        .query_map(params![classroom_id, class_id, rows, columns], |r| {
            let kind = match r.get::<_, String>(3)?.as_str() {
                "seat" => CellKind::Seat,
                "aisle" => CellKind::Aisle,
                _ => CellKind::Unavailable,
            };
            Ok(Position {
                id: r.get(0)?,
                row: r.get(1)?,
                column: r.get(2)?,
                kind,
                desk: r.get(4)?,
            })
        })
        .map_err(error)?
        .collect::<Result<_, _>>()
        .map_err(error)?;
    let mut used_groups: std::collections::HashSet<_> = positions.iter().filter(|p| !p.desk.is_empty())
        .map(|p| p.desk.clone()).collect();
    for row in 1..=rows {
        let seats: Vec<_> = positions.iter().enumerate().filter(|(_, p)| p.row == row && p.kind == CellKind::Seat)
            .map(|(index, _)| index).collect();
        for pair in seats.chunks_exact(2) {
            let (first, second) = (pair[0], pair[1]);
            if positions[first].column + 1 == positions[second].column
                && positions[first].desk.is_empty() && positions[second].desk.is_empty() {
                let mut group = format!("自动同桌 {row}-{}", positions[first].column);
                while !used_groups.insert(group.clone()) { group.push('_'); }
                positions[first].desk = group.clone();
                positions[second].desk = group;
            }
        }
    }
    let mut warnings = Vec::new();
    let mut front_rules = Vec::new();
    let mut soft_rules = Vec::new();
    let mut relations = Vec::new();
    for rule in database::list(db, "rules", Some(class_id))?
        .iter()
        .filter(|r| r["active"] == 1 && r["enabled"] == 1)
    {
        let name = rule["name"].as_str().unwrap_or("未命名规则").to_string();
        let config = serde_json::from_str::<Value>(rule["config"].as_str().unwrap_or(""))
            .unwrap_or(Value::Null);
        let kind = config["type"].as_str().unwrap_or("");
        match (rule["kind"].as_str(), kind) {
            (Some("HARD"), "FRONT_ROW") => {
                let ids = database::rule_students(&config);
                let rows = config["max_row"].as_u64().filter(|n| (1..=50).contains(n));
                match (ids, rows) {
                    (Some(ids), Some(rows)) => for student in ids.into_iter().filter(|id| students.iter().any(|item| item.id == *id)) {
                        front_rules.push(FrontRule { name: name.clone(), student, rows: rows as u16 });
                    },
                    _ => return Err(format!("硬规则「{name}」的学生或前排范围无效")),
                }
            }
            (Some("HARD"), "NOT_DESKMATE" | "NOT_ADJACENT") | (Some("SOFT"), "PREFER_DESKMATE") => {
                let ids = database::rule_students(&config).filter(|ids| ids.len() >= 2)
                    .ok_or_else(|| format!("规则「{name}」至少需要两名学生"))?;
                let ids: Vec<_> = ids.into_iter().filter(|id| students.iter().any(|item| item.id == *id)).collect();
                let relation_kind = match kind {
                    "NOT_DESKMATE" => RelationKind::NotDeskmate,
                    "NOT_ADJACENT" => RelationKind::NotAdjacent,
                    _ => RelationKind::PreferDeskmate,
                };
                let weight = rule["weight"].as_u64().unwrap_or(100) as u32;
                for (index, &first) in ids.iter().enumerate() {
                    for &second in &ids[index + 1..] {
                        relations.push(Relation { first, second, kind: relation_kind, weight });
                    }
                }
            }
            (Some("HARD"), _) => {
                return Err(format!(
                    "无法执行启用的硬规则「{name}」，请检查或停用该规则"
                ))
            }
            _ => {
                let (rule_kind, missing) = match kind {
                    "VISION_FRONT" => (RuleKind::Vision, unknown_vision),
                    "HEIGHT_BACK" => (
                        RuleKind::Height,
                        students
                            .iter()
                            .filter(|s| s.height_tenths.is_none())
                            .count(),
                    ),
                    "GENDER_BALANCE" => (
                        RuleKind::Gender,
                        students.iter().filter(|s| s.gender.is_none()).count(),
                    ),
                    "SCORE_BALANCE" => (
                        RuleKind::Score,
                        students.iter().filter(|s| if use_score_levels { s.score_level.is_none() } else { s.score_rank.is_none() }).count(),
                    ),
                    "DISCIPLINE_SCATTER" => (RuleKind::Discipline, unknown_discipline),
                    _ => {
                        warnings.push(format!("软规则「{name}」无法识别，未参与评分"));
                        continue;
                    }
                };
                let weight = rule["weight"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|&w| w <= 1000);
                let Some(weight) = weight else {
                    warnings.push(format!("软规则「{name}」权重无效，未参与评分"));
                    continue;
                };
                if kind == "SCORE_BALANCE" && missing == students.len() {
                    warnings.push(format!("缺少成绩等级，请先在考试与成绩中同步等级。软规则「{name}」未参与评分"));
                    continue;
                }
                if missing > 0 {
                    warnings.push(format!(
                        "「{name}」有 {missing} 名学生缺少可识别数据，该项对这些学生不计分"
                    ));
                }
                soft_rules.push(SoftRule {
                    name,
                    kind: rule_kind,
                    weight,
                });
            }
        }
    }
    for relation in database::list(db, "relations", Some(class_id))?
        .iter()
        .filter(|r| r["active"] == 1)
    {
        let first = relation["student_id"].as_i64().ok_or("学生关系数据无效")?;
        let second = relation["other_student_id"]
            .as_i64()
            .ok_or("学生关系数据无效")?;
        if !students.iter().any(|s| s.id == first) || !students.iter().any(|s| s.id == second) {
            continue;
        }
        let kind = match relation["kind"].as_str() {
            Some("NOT_DESKMATE") => RelationKind::NotDeskmate,
            Some("NOT_ADJACENT") => RelationKind::NotAdjacent,
            Some("PREFER_DESKMATE") => RelationKind::PreferDeskmate,
            _ => return Err("存在不支持的启用学生关系".into()),
        };
        relations.push(Relation {
            first,
            second,
            kind,
            weight: 100,
        });
    }
    Ok(Snapshot {
        snapshot_version: 1,
        engine_version: seating::ENGINE_VERSION,
        class_id,
        class_name,
        classroom_id,
        classroom_name,
        exam_id,
        exam_name: exam.map(|(name, _)| name),
        rows,
        columns,
        podium_front: podium == "front",
        podium_side: match podium.as_str() { "left" => Some(PodiumSide::Left), "right" => Some(PodiumSide::Right), _ => None },
        use_score_levels,
        students,
        positions,
        front_rules,
        soft_rules,
        relations,
        warnings,
    })
}

pub fn generate(snapshot: Snapshot, seed: u32) -> Result<Plan, String> {
    regenerate(new_draft(snapshot, seed), seed)
}

pub fn new_draft(snapshot: Snapshot, seed: u32) -> Plan {
    Plan {
        id: None,
        name: "排座方案".into(),
        confirmed: false,
        archived: false,
        revision: 0,
        seed,
        source: Source::Offline,
        snapshot,
        assignments: Vec::new(),
        ratings: Vec::new(),
    }
}

pub fn regenerate(mut plan: Plan, seed: u32) -> Result<Plan, String> {
    if plan.confirmed {
        return Err("请先复制为草稿".into());
    }
    let (assignments, ratings) = seating::generate(&plan.snapshot, &plan.assignments, seed)?;
    plan.assignments = assignments;
    plan.ratings = ratings;
    plan.seed = seed;
    plan.source = Source::Offline;
    Ok(plan)
}

pub fn adjust(plan: Plan, action: Adjustment) -> Result<Plan, String> {
    adjust_with_state(plan, action, false)
}

pub fn adjust_with_state(mut plan: Plan, action: Adjustment, editing_active: bool) -> Result<Plan, String> {
    if plan.confirmed && (!editing_active || plan.archived) {
        return Err("请先复制为草稿".into());
    }
    match action {
        Adjustment::Move { from, to } if from != to => {
            let source = plan
                .assignments
                .iter()
                .position(|a| a.position_id == from)
                .ok_or("原座位没有学生")?;
            let target = plan.assignments.iter().position(|a| a.position_id == to);
            if plan.assignments[source].locked || target.is_some_and(|i| plan.assignments[i].locked)
            {
                return Err("请先解除相关座位的锁定".into());
            }
            plan.assignments[source].position_id = to;
            if let Some(index) = target {
                plan.assignments[index].position_id = from;
            }
        }
        Adjustment::Move { .. } => {}
        Adjustment::Lock { position } => {
            let assignment = plan
                .assignments
                .iter_mut()
                .find(|a| a.position_id == position)
                .ok_or("只能锁定已分配学生的座位")?;
            assignment.locked = !assignment.locked;
        }
        Adjustment::BatchLock { positions, locked } => {
            let positions: std::collections::HashSet<_> = positions.into_iter().collect();
            for assignment in plan.assignments.iter_mut().filter(|a| positions.contains(&a.position_id)) {
                assignment.locked = locked;
            }
        }
        Adjustment::Place { student_id, position } => {
            if plan.assignments.iter().any(|a| a.student_id == student_id || a.position_id == position) {
                return Err("学生已入座或目标座位已占用".into());
            }
            plan.assignments.push(Assignment { student_id, position_id: position, locked: false });
        }
        Adjustment::Clear { position } => {
            if plan.assignments.iter().any(|a| a.position_id == position && a.locked) { return Err("请先解除座位锁定".into()); }
            plan.assignments.retain(|a| a.position_id != position);
        }
    }
    plan.ratings = seating::validate_placements(&plan.snapshot, &plan.assignments, false)?;
    Ok(plan)
}

pub fn entries(db: &Connection, class_id: i64) -> Result<Vec<PlanEntry>, String> {
    let mut stmt = db
        .prepare("SELECT id,name,status,archived,source,updated_at FROM seat_plan WHERE class_id=? ORDER BY id DESC")
        .map_err(error)?;
    let rows = stmt
        .query_map([class_id], |r| {
            let source: String = r.get(4)?;
            Ok(PlanEntry {
                id: r.get(0)?,
                name: r.get(1)?,
                confirmed: r.get::<_, String>(2)? == "confirmed",
                archived: r.get(3)?,
                source: if source == "ai" {
                    Source::Ai
                } else {
                    Source::Offline
                },
                updated_at: r.get(5)?,
            })
        })
        .map_err(error)?;
    rows.collect::<Result<_, _>>().map_err(error)
}

pub fn read(db: &Connection, id: i64) -> Result<Plan, String> {
    let (name, status, revision, seed, stored, source, archived): (String, String, u32, u32, String, String, bool) =
        db.query_row(
            "SELECT name,status,revision,seed,snapshot,source,archived FROM seat_plan WHERE id=?",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(error)?
        .ok_or("方案不存在")?;
    let snapshot = serde_json::from_str(&stored).map_err(error)?;
    let mut stmt = db.prepare("SELECT student_id,position_id,locked FROM seat_assignment WHERE plan_id=? ORDER BY student_id").map_err(error)?;
    let assignments = stmt
        .query_map([id], |r| {
            Ok(Assignment {
                student_id: r.get(0)?,
                position_id: r.get(1)?,
                locked: r.get(2)?,
            })
        })
        .map_err(error)?
        .collect::<Result<_, _>>()
        .map_err(error)?;
    let mut stmt = db
        .prepare(
            "SELECT label,score,maximum FROM seat_plan_score WHERE plan_id=? ORDER BY rule_order",
        )
        .map_err(error)?;
    let ratings = stmt
        .query_map([id], |r| {
            Ok(Rating {
                label: r.get(0)?,
                score: r.get(1)?,
                maximum: r.get(2)?,
            })
        })
        .map_err(error)?
        .collect::<Result<_, _>>()
        .map_err(error)?;
    Ok(Plan {
        id: Some(id),
        name,
        confirmed: status == "confirmed",
        archived,
        revision,
        seed,
        source: if source == "ai" {
            Source::Ai
        } else {
            Source::Offline
        },
        snapshot,
        assignments,
        ratings,
    })
}

pub fn save(db: &mut Connection, plan: Plan, confirm: bool) -> Result<Plan, String> {
    persist(db, plan, confirm, false)
}

pub fn save_active(db: &mut Connection, plan: Plan) -> Result<Plan, String> {
    persist(db, plan, true, true)
}

fn persist(db: &mut Connection, mut plan: Plan, confirm: bool, editing_active: bool) -> Result<Plan, String> {
    if editing_active && (plan.id.is_none() || !plan.confirmed || plan.archived) {
        return Err("请打开当前启用的方案后修改".into());
    }
    if plan.confirmed && !editing_active {
        return Err("请先复制为草稿".into());
    }
    plan.name = plan.name.trim().to_string();
    if !(1..=100).contains(&plan.name.chars().count()) {
        return Err("方案名称须为 1 至 100 个字符".into());
    }
    if confirm && plan.assignments.is_empty() { return Err("方案没有座位分配，不能确认".into()); }
    plan.ratings = seating::validate_placements(&plan.snapshot, &plan.assignments, false)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    let status = if confirm { "confirmed" } else { "draft" };
    let id = if let Some(id) = plan.id {
        let old = read(&tx, id)?;
        if old.confirmed != editing_active || old.archived || old.revision != plan.revision || old.snapshot != plan.snapshot {
            return Err("方案已确认或已被其他操作修改，请重新打开".into());
        }
        tx.execute("UPDATE seat_plan SET name=?,status=?,revision=revision+1,seed=?,source=?,updated_at=CURRENT_TIMESTAMP WHERE id=?",
            params![plan.name,status,plan.seed,plan.source.as_str(),id]).map_err(error)?;
        tx.execute("DELETE FROM seat_assignment WHERE plan_id=?", [id])
            .map_err(error)?;
        tx.execute("DELETE FROM seat_plan_score WHERE plan_id=?", [id])
            .map_err(error)?;
        id
    } else {
        let current = snapshot(
            &tx,
            plan.snapshot.class_id,
            plan.snapshot.classroom_id,
            plan.snapshot.exam_id,
        )?;
        if current != plan.snapshot {
            return Err("排座资料已变化，请重新生成方案".into());
        }
        insert(&tx, &plan, status)?
    };
    write_items(&tx, id, &plan)?;
    if confirm && !editing_active {
        activate(&tx, plan.snapshot.class_id, id)?;
    }
    let stored = read(&tx, id)?;
    tx.commit().map_err(error)?;
    Ok(stored)
}

fn activate(db: &Connection, class_id: i64, id: i64) -> Result<(), String> {
    db.execute("UPDATE seat_plan SET archived=(id<>?2) WHERE class_id=?1 AND status='confirmed' AND (archived<>(id<>?2))",
        params![class_id, id]).map_err(error)?;
    Ok(())
}

pub fn activate_archived(db: &mut Connection, id: i64) -> Result<Plan, String> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
    let plan = read(&tx, id)?;
    if !plan.confirmed || plan.assignments.is_empty() { return Err("只有已确认且有座位分配的方案可以启用".into()); }
    activate(&tx, plan.snapshot.class_id, id)?;
    let stored = read(&tx, id)?;
    tx.commit().map_err(error)?;
    Ok(stored)
}

fn insert(db: &Connection, plan: &Plan, status: &str) -> Result<i64, String> {
    db.execute(
        "INSERT INTO seat_plan(class_id,name,status,revision,snapshot,seed,source) VALUES(?,?,?,1,?,?,?)",
        params![
            plan.snapshot.class_id,
            plan.name,
            status,
            serde_json::to_string(&plan.snapshot).map_err(error)?,
            plan.seed,
            plan.source.as_str()
        ],
    )
    .map_err(error)?;
    Ok(db.last_insert_rowid())
}

fn write_items(db: &Connection, id: i64, plan: &Plan) -> Result<(), String> {
    let mut stmt = db
        .prepare(
            "INSERT INTO seat_assignment(plan_id,student_id,position_id,locked) VALUES(?,?,?,?)",
        )
        .map_err(error)?;
    for a in &plan.assignments {
        stmt.execute(params![id, a.student_id, a.position_id, a.locked])
            .map_err(error)?;
    }
    let mut stmt = db
        .prepare(
            "INSERT INTO seat_plan_score(plan_id,rule_order,label,score,maximum) VALUES(?,?,?,?,?)",
        )
        .map_err(error)?;
    for (index, r) in plan.ratings.iter().enumerate() {
        stmt.execute(params![id, index as i64, r.label, r.score, r.maximum])
            .map_err(error)?;
    }
    Ok(())
}

pub fn copy(db: &mut Connection, id: i64) -> Result<Plan, String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    let mut plan = read(&tx, id)?;
    seating::validate_placements(&plan.snapshot, &plan.assignments, false)?;
    plan.name = format!("{}（副本）", plan.name.chars().take(96).collect::<String>());
    let id = insert(&tx, &plan, "draft")?;
    write_items(&tx, id, &plan)?;
    let stored = read(&tx, id)?;
    tx.commit().map_err(error)?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        database::initialize(&mut db).unwrap();
        db.execute_batch(r#"INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(1,'快照班','','','');
            INSERT INTO seat_classroom(id,class_id,name,rows,columns,podium) VALUES(1,1,'测试教室',2,3,'back');
            INSERT INTO seat_student(id,class_id,student_no,name,eyesight) VALUES(1,1,'TEST-001','测试甲','近视'),(2,1,'TEST-002','测试乙','正常');
            INSERT INTO seat_position(id,class_id,classroom_id,row_number,column_number,kind,desk_group)
              VALUES(1,1,1,1,1,'seat','一组'),(2,1,1,1,2,'aisle',''),(3,1,1,1,3,'seat',''),
                    (4,1,1,2,1,'seat','一组'),(5,1,1,2,2,'seat',''),(6,1,1,2,3,'seat','');
            INSERT INTO seat_rule(class_id,name,kind,weight,config) VALUES
              (1,'前排优先','HARD',1,'{"type":"FRONT_ROW","student_id":1,"max_row":1}'),
              (1,'视力规则','SOFT',2,'{"type":"VISION_FRONT"}');
            INSERT INTO seat_student_relation(class_id,student_id,other_student_id,kind) VALUES(1,1,2,'NOT_ADJACENT');"#).unwrap();
        db
    }

    #[test]
    fn hard_constraints_use_podium_and_direct_neighbors() {
        let db = fixture();
        let snap = snapshot(&db, 1, 1, None).unwrap();
        let assignments = |a, b| {
            vec![
                Assignment {
                    student_id: 1,
                    position_id: a,
                    locked: false,
                },
                Assignment {
                    student_id: 2,
                    position_id: b,
                    locked: false,
                },
            ]
        };
        assert!(seating::validate(&snap, &assignments(1, 3))
            .unwrap_err()
            .contains("前 1 排"));
        assert!(seating::validate(&snap, &assignments(4, 1))
            .unwrap_err()
            .contains("不能相邻"));
        assert!(seating::validate(&snap, &assignments(4, 5))
            .unwrap_err()
            .contains("不能相邻"));
        assert!(seating::validate(&snap, &assignments(5, 1)).is_ok());
        assert!(seating::validate(&snap, &assignments(4, 6)).is_ok());
        let placed = seating::generate(&snap, &[], 17).unwrap();
        assert_eq!(placed, seating::generate(&snap, &[], 17).unwrap());
        let mut locked = placed.0;
        locked[0].locked = true;
        assert_eq!(
            seating::generate(&snap, &locked, 19).unwrap().0[0],
            locked[0]
        );
        locked[0].position_id = 1;
        assert!(seating::generate(&snap, &locked, 19)
            .unwrap_err()
            .contains("锁座冲突"));
        let mut same_desk = snap.clone();
        same_desk.relations[0].kind = RelationKind::NotDeskmate;
        assert!(seating::validate(&same_desk, &assignments(4, 1))
            .unwrap_err()
            .contains("不能同桌"));
        assert!(seating::validate(&same_desk, &assignments(4, 5)).is_ok());
        same_desk.positions.retain(|position| position.id == 4);
        assert!(seating::generate(&same_desk, &[], 17)
            .unwrap_err()
            .contains("可用座位不足"));
    }

    #[test]
    fn history_copy_and_failed_write_preserve_the_original() {
        let mut db = fixture();
        let preview = generate(snapshot(&db, 1, 1, None).unwrap(), 1).unwrap();
        let draft = save(&mut db, preview, false).unwrap();
        let id = draft.id.unwrap();
        db.execute_batch(
            "CREATE TRIGGER reject_rating BEFORE INSERT ON seat_plan_score
            BEGIN SELECT RAISE(ABORT,'test write failure'); END;",
        )
        .unwrap();
        let mut edited = read(&db, id).unwrap();
        edited.name = "第二版".into();
        assert!(save(&mut db, edited, true).is_err());
        assert_eq!(read(&db, id).unwrap().revision, 1);
        assert_eq!(read(&db, id).unwrap().assignments, draft.assignments);
        db.execute_batch("DROP TRIGGER reject_rating;").unwrap();
        let confirmed = save(&mut db, draft, true).unwrap();
        let immutable = read(&db, id).unwrap();
        assert!(save(&mut db, immutable, false).is_err());
        db.execute_batch(
            "UPDATE seat_student SET name='改名后' WHERE id=1;
            UPDATE seat_classroom SET podium='front' WHERE id=1;
            UPDATE seat_rule SET active=0;",
        )
        .unwrap();
        let copy = copy(&mut db, id).unwrap();
        assert!(!copy.confirmed);
        assert_ne!(copy.id, confirmed.id);
        assert_eq!(copy.snapshot, confirmed.snapshot);
        assert_eq!(copy.snapshot.students[0].name, "测试甲");
        let student_file = crate::plan_export::project(&copy, false);
        let json = serde_json::to_string(&student_file).unwrap();
        assert!(
            !json.contains("TEST-001") && !json.contains("ratings") && !json.contains("locked")
        );
        let teacher_file = crate::plan_export::project(&copy, true);
        let json = serde_json::to_string(&teacher_file).unwrap();
        assert!(json.contains("TEST-001") && json.contains("视力规则"));
        assert!(crate::plan_export::workbook(&student_file)
            .unwrap()
            .starts_with(b"PK"));
    }

    #[test]
    fn right_podium_group_rules_and_manual_plan_lifecycle() {
        let mut db = fixture();
        db.execute_batch("UPDATE seat_classroom SET podium='right';
            UPDATE seat_position SET desk_group='';
            UPDATE seat_position SET kind='seat' WHERE id=2;
            UPDATE seat_rule SET active=0;
            UPDATE seat_student_relation SET active=0;
            UPDATE seat_student SET eyesight='轻度',discipline='关注' WHERE id=1;
            INSERT INTO seat_rule(class_id,name,kind,weight,config) VALUES
            (1,'禁止同桌','HARD',100,'{\"type\":\"NOT_DESKMATE\",\"student_ids\":[1,2]}');").unwrap();
        let snapshot = snapshot(&db, 1, 1, None).unwrap();
        assert_eq!(snapshot.podium(), "right");
        assert_eq!(snapshot.depth(snapshot.positions.iter().find(|position| position.column == 3).unwrap()), 1);
        assert!(snapshot.students[0].poor_vision && snapshot.students[0].discipline_attention);
        assert!(snapshot.positions[0].desk.starts_with("自动同桌"));
        let mut draft = save(&mut db, new_draft(snapshot, 1), false).unwrap();
        let original_id = draft.id.unwrap();
        assert!(save(&mut db, draft.clone(), true).is_err());
        draft = adjust(draft, Adjustment::Place { student_id: 1, position: 1 }).unwrap();
        assert!(adjust(draft.clone(), Adjustment::Place { student_id: 2, position: 2 }).is_err());
        draft = adjust(draft, Adjustment::Place { student_id: 2, position: 3 }).unwrap();
        draft = adjust(draft, Adjustment::BatchLock { positions: vec![1, 3], locked: true }).unwrap();
        let published = save(&mut db, draft, true).unwrap();
        assert!(published.assignments.iter().all(|item| item.locked));
        let editing = adjust_with_state(published.clone(), Adjustment::Lock { position: 3 }, true).unwrap();
        let edited = save_active(&mut db, editing.clone()).unwrap();
        assert!(!edited.assignments.iter().find(|item| item.position_id == 3).unwrap().locked);
        assert!(save_active(&mut db, editing).is_err());
        let copy = copy(&mut db, original_id).unwrap();
        let current = save(&mut db, copy, true).unwrap();
        assert!(read(&db, original_id).unwrap().archived);
        assert!(!activate_archived(&mut db, original_id).unwrap().archived);
        assert!(read(&db, current.id.unwrap()).unwrap().archived);
    }
}
