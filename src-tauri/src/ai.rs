use crate::{
    ai_settings::{check_key, Profile, ProfileList, Settings, SettingsStore, View},
    plan_commands,
    plans::{self, Plan, Source},
    seating::{self, Assignment, CellKind, RelationKind},
    Database,
};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::State;
use tokio::sync::oneshot;
use uuid::Uuid;

const OUTPUT_LIMIT: usize = 1024 * 1024;
const PROMPT: &str = "Assign each student id to one unique seat id. Respect all locked placements, front restrictions, and student relations. Use only supplied ids. Return a JSON object with only assignments, an array of objects with exactly student and seat string keys. Return the JSON object directly.";

#[derive(Serialize)]
struct RequestData {
    students: Vec<String>,
    seats: Vec<Seat>,
    podium: &'static str,
    desk_groups: Vec<Vec<String>>,
    adjacent: Vec<[String; 2]>,
    locked: Vec<Pair>,
    front: Vec<Front>,
    relations: Vec<HardRelation>,
}

#[derive(Serialize)]
struct Seat {
    id: String,
    row: u16,
    column: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    student: String,
    seat: String,
}

#[derive(Serialize)]
struct Front {
    student: String,
    allowed_seats: Vec<String>,
}

#[derive(Serialize)]
struct HardRelation {
    students: [String; 2],
    kind: &'static str,
}

struct Input {
    plan: Plan,
    data: RequestData,
    student_ids: HashMap<String, i64>,
    seat_ids: HashMap<String, i64>,
}

impl Input {
    fn new(plan: Plan) -> Result<Self, String> {
        if plan.confirmed {
            return Err("请先复制已确认方案，再用 AI 调整".into());
        }
        if !plan.assignments.is_empty() {
            seating::validate(&plan.snapshot, &plan.assignments)?;
        }
        let snapshot = &plan.snapshot;
        let seats: Vec<_> = snapshot
            .positions
            .iter()
            .filter(|p| p.kind == CellKind::Seat)
            .collect();
        if snapshot.students.is_empty() || seats.len() < snapshot.students.len() {
            return Err("请先检查学生名单和可用座位数量".into());
        }
        let new_id = || Uuid::new_v4().simple().to_string();
        let students: HashMap<_, _> = snapshot.students.iter().map(|s| (s.id, new_id())).collect();
        let positions: HashMap<_, _> = seats.iter().map(|s| (s.id, new_id())).collect();
        let seat_grid: HashMap<_, _> = seats.iter().map(|p| ((p.row, p.column), p.id)).collect();
        let mut desks = std::collections::BTreeMap::<&str, Vec<String>>::new();
        let mut adjacent = Vec::new();
        for p in &seats {
            for next in [
                (p.row.saturating_add(1), p.column),
                (p.row, p.column.saturating_add(1)),
            ] {
                if let Some(other) = seat_grid.get(&next) {
                    adjacent.push([positions[&p.id].clone(), positions[other].clone()]);
                }
            }
            if !p.desk.is_empty() {
                desks
                    .entry(&p.desk)
                    .or_default()
                    .push(positions[&p.id].clone());
            }
        }
        let locked = plan
            .assignments
            .iter()
            .filter(|a| a.locked)
            .map(|a| Pair {
                student: students[&a.student_id].clone(),
                seat: positions[&a.position_id].clone(),
            })
            .collect();
        let front = snapshot
            .front_rules
            .iter()
            .map(|rule| {
                let student = students
                    .get(&rule.student)
                    .ok_or("前排规则引用了不在当前名单的学生")?
                    .clone();
                let allowed_seats = seats
                    .iter()
                    .filter(|p| snapshot.depth(p) <= rule.rows)
                    .map(|p| positions[&p.id].clone())
                    .collect::<Vec<_>>();
                if allowed_seats.is_empty() {
                    return Err("前排规则没有可用座位");
                }
                Ok(Front {
                    student,
                    allowed_seats,
                })
            })
            .collect::<Result<_, _>>()?;
        let relations = snapshot
            .relations
            .iter()
            .filter_map(|relation| {
                let kind = match relation.kind {
                    RelationKind::NotAdjacent => "not_adjacent",
                    RelationKind::NotDeskmate => "not_deskmate",
                    RelationKind::PreferDeskmate => return None,
                };
                Some(HardRelation {
                    students: [
                        students.get(&relation.first)?.clone(),
                        students.get(&relation.second)?.clone(),
                    ],
                    kind,
                })
            })
            .collect();
        let mut shuffled: Vec<_> = students.values().cloned().collect();
        shuffled.sort_unstable();
        let data = RequestData {
            students: shuffled,
            seats: seats
                .iter()
                .map(|p| Seat {
                    id: positions[&p.id].clone(),
                    row: p.row,
                    column: p.column,
                })
                .collect(),
            podium: snapshot.podium(),
            desk_groups: desks
                .into_values()
                .filter(|group| group.len() > 1)
                .collect(),
            adjacent,
            locked,
            front,
            relations,
        };
        Ok(Self {
            plan,
            data,
            student_ids: students.into_iter().map(|(k, v)| (v, k)).collect(),
            seat_ids: positions.into_iter().map(|(k, v)| (v, k)).collect(),
        })
    }

    fn accept(&self, text: &str) -> Result<Plan, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Answer {
            assignments: Vec<Pair>,
        }
        let answer: Answer = serde_json::from_str(text).map_err(|_| "AI 响应格式无效")?;
        if answer.assignments.len() != self.student_ids.len() {
            return Err("AI 响应未完整分配学生".into());
        }
        let locked: HashMap<_, _> = self
            .plan
            .assignments
            .iter()
            .filter(|a| a.locked)
            .map(|a| (a.student_id, a.position_id))
            .collect();
        let mut distinct_students = HashSet::new();
        let mut distinct_seats = HashSet::new();
        let mut assignments = Vec::new();
        for pair in answer.assignments {
            let &student_id = self
                .student_ids
                .get(&pair.student)
                .ok_or("AI 响应含未知学生编号")?;
            let &position_id = self
                .seat_ids
                .get(&pair.seat)
                .ok_or("AI 响应含未知座位编号")?;
            if !distinct_students.insert(student_id) || !distinct_seats.insert(position_id) {
                return Err("AI 响应重复分配学生或座位".into());
            }
            let fixed = locked.get(&student_id);
            if fixed.is_some_and(|&p| p != position_id) {
                return Err("AI 响应违反锁座".into());
            }
            assignments.push(Assignment {
                student_id,
                position_id,
                locked: fixed.is_some(),
            });
        }
        assignments.sort_by_key(|a| a.student_id);
        let ratings = seating::validate(&self.plan.snapshot, &assignments)
            .map_err(|_| "AI 响应违反本地硬约束")?;
        let mut plan = self.plan.clone();
        plan.assignments = assignments;
        plan.ratings = ratings;
        plan.source = Source::Ai;
        Ok(plan)
    }

    fn message(&self) -> Result<String, String> {
        let text = serde_json::to_string(&self.data).map_err(|_| "无法编码 AI 请求")?;
        if text.len() > OUTPUT_LIMIT {
            return Err("本次请求超出 AI 数据大小限制".into());
        }
        Ok(text)
    }
}

enum Stage {
    Ready,
    Running(oneshot::Sender<()>),
    Failed,
}

struct Attempt {
    id: String,
    profile: Profile,
    input: Arc<Input>,
    stage: Stage,
}

pub struct AiState {
    store: SettingsStore,
    pending: Mutex<Option<Attempt>>,
}

impl AiState {
    pub fn new(directory: PathBuf, identifier: String) -> Self {
        Self {
            store: SettingsStore::new(directory, identifier),
            pending: Mutex::new(None),
        }
    }
}

#[derive(Serialize)]
pub struct Preview {
    id: String,
    endpoint: String,
    model: String,
    payload: serde_json::Value,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunResult {
    Success { plan: Box<Plan> },
    Failed { reason: String },
    Cancelled,
}

async fn complete(
    profile: &Profile,
    key: Option<String>,
    system: &str,
    message: String,
) -> Result<String, String> {
    let settings = &profile.settings;
    let client = client(settings)?;
    let mut body = serde_json::json!({ "model":settings.model,"stream":false,"messages":[
        {"role":"system","content":system},{"role":"user","content":message}
    ]});
    let model = settings.model.to_lowercase();
    let reasoning = ["o1", "o3", "o4", "gpt-5"].iter().any(|prefix| model.starts_with(prefix));
    if let Some(value) = settings.temperature.filter(|_| !reasoning) { body["temperature"] = value.into(); }
    if let Some(value) = settings.max_tokens {
        body[if reasoning { "max_completion_tokens" } else { "max_tokens" }] = value.into();
    }
    if let Some(effort) = settings.reasoning_effort.as_deref() {
        match (profile.provider.as_str(), effort) {
            ("ZHIPU", effort) => {
                let glm = model.starts_with("glm-5.3");
                body["thinking"] = serde_json::json!({"type": if glm || effort != "off" { "enabled" } else { "disabled" }});
                if glm { body["reasoning_effort"] = (if matches!(effort, "off" | "low") { "low" } else { "high" }).into(); }
            }
            ("QWEN", effort) => {
                body["enable_thinking"] = (effort != "off").into();
                if effort != "off" { body["reasoning_effort"] = effort.into(); }
            }
            ("SILICONFLOW", "off") if model.contains("qwen3") => body["enable_thinking"] = false.into(),
            (_, "off") if reasoning => body["reasoning_effort"] = (if model.starts_with("gpt-5") { "minimal" } else { "low" }).into(),
            (_, "off") => body["thinking"] = serde_json::json!({"type":"disabled"}),
            (_, effort) => body["reasoning_effort"] = effort.into(),
        }
    }
    let mut request = client.post(&settings.endpoint).json(&body);
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let bytes = response_bytes(request).await?;
    #[derive(Deserialize)]
    struct Response {
        choices: Vec<Choice>,
    }
    #[derive(Deserialize)]
    struct Choice {
        finish_reason: Option<String>,
        message: Message,
    }
    #[derive(Deserialize)]
    struct Message {
        content: Option<String>,
    }
    let answer: Response = serde_json::from_slice(&bytes).map_err(|_| "AI 响应格式无效")?;
    let [choice]: [Choice; 1] = answer
        .choices
        .try_into()
        .map_err(|_| "AI 响应未返回唯一结果")?;
    if choice.finish_reason.as_deref() != Some("stop") {
        return Err(if choice.finish_reason.as_deref() == Some("length") {
            "AI 响应被截断"
        } else {
            "AI 服务未正常完成生成"
        }
        .into());
    }
    choice.message.content.filter(|s| !s.is_empty()).ok_or_else(|| "AI 响应内容为空".into())
}

fn client(settings: &Settings) -> Result<Client, String> {
    let timeout = Duration::from_secs(settings.timeout_seconds);
    Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| "无法初始化 AI 网络连接".into())
}

async fn response_bytes(request: reqwest::RequestBuilder) -> Result<Vec<u8>, String> {
    let network_error = |error: reqwest::Error| {
        if error.is_timeout() {
            "AI 请求超时"
        } else {
            "无法连接 AI 服务或响应连接中断"
        }
    };
    let mut response = request.send().await.map_err(network_error)?;
    if !response.status().is_success() {
        return Err(match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "AI 服务拒绝认证".into(),
            StatusCode::TOO_MANY_REQUESTS => "AI 服务限流".into(),
            code if code.is_redirection() => "AI 服务要求重定向，未向新地址发送数据".into(),
            code => format!("AI 服务返回 HTTP {}", code.as_u16()),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > OUTPUT_LIMIT {
            return Err("AI 响应超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[tauri::command]
pub fn ai_settings(ai: State<'_, AiState>) -> Result<View, String> {
    ai.store.view()
}

#[tauri::command]
pub fn save_ai_settings(
    ai: State<'_, AiState>,
    settings: Settings,
    key: Option<String>,
    forget_key: bool,
) -> Result<View, String> {
    ai.store.save(settings, key, forget_key)
}

#[tauri::command]
pub async fn test_ai_connection(ai: State<'_, AiState>) -> Result<(), String> {
    let profile = ai.store.selected(None)?.ok_or("请先保存 AI 设置")?;
    let key = ai.store.key(profile.id, &profile.settings)?;
    test_profile(&profile, key).await
}

async fn test_profile(profile: &Profile, key: Option<String>) -> Result<(), String> {
    complete(
        profile,
        key,
        "Reply with a short confirmation.",
        "Connection check.".into(),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub fn list_ai_profiles(ai: State<'_, AiState>) -> Result<ProfileList, String> {
    ai.store.list()
}

#[tauri::command]
pub fn save_ai_profile(ai: State<'_, AiState>, profile: Profile, key: Option<String>, forget_key: bool, make_default: bool) -> Result<ProfileList, String> {
    ai.store.save_profile(profile, key, forget_key, make_default)
}

#[tauri::command]
pub fn change_ai_profile(ai: State<'_, AiState>, id: u64, remove: bool) -> Result<ProfileList, String> {
    ai.store.change_profile(id, remove)
}

#[tauri::command]
pub async fn test_ai_profile(ai: State<'_, AiState>, mut profile: Profile, key: Option<String>, use_saved_key: bool) -> Result<(), String> {
    profile.settings = profile.settings.checked()?;
    check_key(key.as_deref())?;
    let key = match key {
        None if use_saved_key => ai.store.key(profile.id, &profile.settings)?,
        key => key,
    };
    test_profile(&profile, key).await
}

#[tauri::command]
pub async fn fetch_ai_models(ai: State<'_, AiState>, mut profile: Profile, key: Option<String>, use_saved_key: bool) -> Result<Vec<String>, String> {
    profile.settings.model = "model-discovery".into();
    let settings = profile.settings.checked()?;
    check_key(key.as_deref())?;
    let key = match key {
        None if use_saved_key => ai.store.key(profile.id, &settings)?,
        key => key,
    };
    let base = settings.endpoint.strip_suffix("/chat/completions").ok_or("服务地址无效")?;
    let mut request = client(&settings)?.get(format!("{base}/models"));
    if let Some(key) = key { request = request.bearer_auth(key); }
    #[derive(Deserialize)]
    struct ModelList { data: Vec<Model> }
    #[derive(Deserialize)]
    struct Model { id: String }
    let bytes = response_bytes(request).await?;
    let response: ModelList = serde_json::from_slice(&bytes).map_err(|_| "模型列表格式无效")?;
    let mut models: Vec<_> = response.data.into_iter().map(|model| model.id)
        .filter(|id| !id.trim().is_empty() && id.len() <= 100 && !id.chars().any(char::is_control)).collect();
    models.sort();
    models.dedup();
    Ok(models)
}

#[tauri::command]
pub fn prepare_ai(
    ai: State<'_, AiState>,
    database: State<'_, Database>,
    class_id: i64,
    classroom_id: i64,
    exam_id: Option<i64>,
    seed: u32,
    prior: Option<Plan>,
    profile_id: Option<u64>,
) -> Result<Preview, String> {
    drop(database.connection()?);
    let profile = ai.store.selected(profile_id)?.ok_or("请选择已启用的 AI 模型配置")?;
    let mut pending = ai.pending.lock().map_err(|_| "AI 状态不可用")?;
    if pending
        .as_ref()
        .is_some_and(|p| matches!(p.stage, Stage::Running(_)))
    {
        return Err("已有 AI 请求正在运行".into());
    }
    let mut plan = match prior {
        Some(plan)
            if !plan.confirmed
                && plan.snapshot.class_id == class_id
                && plan.snapshot.classroom_id == classroom_id =>
        {
            plan
        }
        Some(_) => return Err("请选择与当前排座资料一致的可编辑草稿".into()),
        None => plans::new_draft(
            plan_commands::read_snapshot(database, class_id, classroom_id, exam_id)?,
            seed,
        ),
    };
    plan.seed = seed;
    let input = Arc::new(Input::new(plan)?);
    input.message()?;
    let payload = serde_json::to_value(&input.data).map_err(|_| "无法生成发送预览")?;
    let id = Uuid::new_v4().to_string();
    let preview = Preview {
        id: id.clone(),
        endpoint: profile.settings.endpoint.clone(),
        model: profile.settings.model.clone(),
        payload,
    };
    *pending = Some(Attempt {
        id,
        profile,
        input,
        stage: Stage::Ready,
    });
    Ok(preview)
}

#[tauri::command]
pub async fn run_ai(ai: State<'_, AiState>, id: String) -> Result<RunResult, String> {
    let (input, profile, receiver) = {
        let mut pending = ai.pending.lock().map_err(|_| "AI 状态不可用")?;
        let attempt = pending
            .as_mut()
            .filter(|p| p.id == id && matches!(p.stage, Stage::Ready))
            .ok_or("请求已结束，请重新预览")?;
        let (sender, receiver) = oneshot::channel();
        attempt.stage = Stage::Running(sender);
        (
            Arc::clone(&attempt.input),
            attempt.profile.clone(),
            receiver,
        )
    };
    let request = async {
        let body = input.message()?;
        let key = ai.store.key(profile.id, &profile.settings)?;
        let response = complete(&profile, key, PROMPT, body).await?;
        input.accept(&response)
    };
    let result = tokio::select! {
        biased;
        _ = receiver => return Ok(RunResult::Cancelled),
        result = request => result,
    };
    let mut pending = ai.pending.lock().map_err(|_| "AI 状态不可用")?;
    let Some(attempt) = pending.as_mut().filter(|p| p.id == id) else {
        return Ok(RunResult::Cancelled);
    };
    match result {
        Ok(plan) => {
            *pending = None;
            Ok(RunResult::Success {
                plan: Box::new(plan),
            })
        }
        Err(reason) => {
            attempt.stage = Stage::Failed;
            Ok(RunResult::Failed { reason })
        }
    }
}

#[tauri::command]
pub fn cancel_ai(ai: State<'_, AiState>, id: String) -> Result<(), String> {
    let mut pending = ai.pending.lock().map_err(|_| "AI 状态不可用")?;
    if pending.as_ref().is_some_and(|p| p.id == id) {
        let attempt = pending.take().unwrap();
        if let Stage::Running(sender) = attempt.stage {
            let _ = sender.send(());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn fallback_ai(ai: State<'_, AiState>, id: String) -> Result<Plan, String> {
    let plan = {
        let mut pending = ai.pending.lock().map_err(|_| "AI 状态不可用")?;
        if !pending
            .as_ref()
            .is_some_and(|p| p.id == id && matches!(p.stage, Stage::Failed))
        {
            return Err("本次请求没有可用的离线回退".into());
        }
        pending.take().unwrap().input.plan.clone()
    };
    let seed = plan.seed;
    tauri::async_runtime::spawn_blocking(move || plans::regenerate(plan, seed))
        .await
        .map_err(|_| "离线排座任务中断")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_projection_and_returned_ids_enforce_the_privacy_and_lock_boundary() {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::initialize(&mut db).unwrap();
        db.execute_batch(r#"
            INSERT INTO seat_class(id,name,school_stage,grade,term) VALUES(1,'私有班名','','','');
            INSERT INTO seat_classroom(id,class_id,name,rows,columns,podium) VALUES(1,1,'私有教室',1,3,'front');
            INSERT INTO seat_student(id,class_id,student_no,name,height,notes) VALUES(101,1,'RAW-ID-A','私有姓名甲',168,'私有备注'),(102,1,'RAW-ID-B','私有姓名乙',155,'');
            INSERT INTO seat_position(id,class_id,classroom_id,row_number,column_number,kind,desk_group)
                VALUES(1,1,1,1,1,'seat','私有分组'),(2,1,1,1,2,'seat','私有分组'),(3,1,1,1,3,'seat','');
            INSERT INTO seat_student_relation(class_id,student_id,other_student_id,kind) VALUES(1,101,102,'NOT_ADJACENT');
        "#).unwrap();
        let mut plan = plans::generate(plans::snapshot(&db, 1, 1, None).unwrap(), 11).unwrap();
        plan.assignments[0].locked = true;
        let a = Input::new(plan.clone()).unwrap();
        let b = Input::new(plan.clone()).unwrap();
        assert!(a
            .student_ids
            .keys()
            .all(|id| !b.student_ids.contains_key(id)));
        let body = a.message().unwrap();
        assert!(!body.contains("私有") && !body.contains("RAW-ID"));
        assert_eq!(a.data.relations.len(), 1);
        assert_eq!(a.data.locked.len(), 1);
        let mut pairs: Vec<_> = plan
            .assignments
            .iter()
            .map(|assignment| Pair {
                student: a
                    .student_ids
                    .iter()
                    .find(|(_, id)| **id == assignment.student_id)
                    .unwrap()
                    .0
                    .clone(),
                seat: a
                    .seat_ids
                    .iter()
                    .find(|(_, id)| **id == assignment.position_id)
                    .unwrap()
                    .0
                    .clone(),
            })
            .collect();
        let json = serde_json::json!({"assignments": pairs});
        assert_eq!(a.accept(&json.to_string()).unwrap().source, Source::Ai);
        pairs[0].seat = a
            .seat_ids
            .iter()
            .find(|(_, id)| !plan.assignments.iter().any(|r| r.position_id == **id))
            .unwrap()
            .0
            .clone();
        assert!(a
            .accept(&serde_json::json!({"assignments":pairs}).to_string())
            .unwrap_err()
            .contains("锁座"));
    }
}
