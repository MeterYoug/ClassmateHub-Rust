use crate::{
    plan_export,
    plans::{self, Adjustment, Plan, PlanEntry},
    Database,
};
use tauri::{
    ipc::{InvokeBody, Request},
    State, WebviewWindow,
};

pub(crate) fn read_snapshot(
    database: State<'_, Database>,
    class_id: i64,
    classroom_id: i64,
    exam_id: Option<i64>,
) -> Result<crate::seating::Snapshot, String> {
    let mut db = database.connection()?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    plans::snapshot(&tx, class_id, classroom_id, exam_id)
}

#[tauri::command]
pub fn inspect_plan_inputs(
    database: State<'_, Database>,
    class_id: i64,
    classroom_id: i64,
    exam_id: Option<i64>,
) -> Result<Vec<String>, String> {
    Ok(read_snapshot(database, class_id, classroom_id, exam_id)?.warnings)
}

#[tauri::command]
pub fn new_manual_plan(
    database: State<'_, Database>, class_id: i64, classroom_id: i64, exam_id: Option<i64>, seed: u32,
) -> Result<Plan, String> {
    let snapshot = read_snapshot(database, class_id, classroom_id, exam_id)?;
    crate::seating::validate_placements(&snapshot, &[], false)?;
    Ok(plans::new_draft(snapshot, seed))
}

#[tauri::command]
pub fn activate_plan(database: State<'_, Database>, id: i64) -> Result<Plan, String> {
    let mut db = database.connection()?;
    plans::activate_archived(&mut db, id)
}

#[tauri::command]
pub async fn generate_plan(
    database: State<'_, Database>,
    class_id: i64,
    classroom_id: i64,
    exam_id: Option<i64>,
    seed: u32,
) -> Result<Plan, String> {
    let snapshot = read_snapshot(database, class_id, classroom_id, exam_id)?;
    tauri::async_runtime::spawn_blocking(move || plans::generate(snapshot, seed))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn regenerate_plan(plan: Plan, seed: u32) -> Result<Plan, String> {
    tauri::async_runtime::spawn_blocking(move || plans::regenerate(plan, seed))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn adjust_plan(plan: Plan, adjustment: Adjustment, editing_active: Option<bool>) -> Result<Plan, String> {
    if editing_active == Some(true) { plans::adjust_with_state(plan, adjustment, true) }
    else { plans::adjust(plan, adjustment) }
}

#[tauri::command]
pub fn save_active_plan(database: State<'_, Database>, plan: Plan) -> Result<Plan, String> {
    let mut db = database.connection()?;
    plans::save_active(&mut db, plan)
}

#[tauri::command]
pub fn list_plans(database: State<'_, Database>, class_id: i64) -> Result<Vec<PlanEntry>, String> {
    let db = database.connection()?;
    plans::entries(&db, class_id)
}

#[tauri::command]
pub fn open_plan(database: State<'_, Database>, id: i64) -> Result<Plan, String> {
    let mut db = database.connection()?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    plans::read(&tx, id)
}

#[tauri::command]
pub fn save_plan(database: State<'_, Database>, plan: Plan, confirm: bool) -> Result<Plan, String> {
    let mut db = database.connection()?;
    plans::save(&mut db, plan, confirm)
}

#[tauri::command]
pub fn copy_plan(database: State<'_, Database>, id: i64) -> Result<Plan, String> {
    let mut db = database.connection()?;
    plans::copy(&mut db, id)
}

#[tauri::command]
pub fn plan_export_sheet(
    database: State<'_, Database>,
    id: i64,
    teacher: bool,
) -> Result<plan_export::Sheet, String> {
    Ok(plan_export::project(&open_plan(database, id)?, teacher))
}

#[tauri::command]
pub async fn export_plan_workbook<R: tauri::Runtime>(
    window: WebviewWindow<R>,
    database: State<'_, Database>,
    id: i64,
    teacher: bool,
) -> Result<bool, String> {
    let sheet = plan_export_sheet(database, id, teacher)?;
    tauri::async_runtime::spawn_blocking(move || {
        plan_export::save_file(window, plan_export::workbook(&sheet)?, "xlsx")
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn export_plan_image<R: tauri::Runtime>(
    window: WebviewWindow<R>,
    request: Request<'_>,
) -> Result<bool, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("导出数据格式无效".into());
    };
    let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"%PDF-") {
        "pdf"
    } else {
        return Err("仅支持 PNG 或 PDF 导出".into());
    };
    if bytes.len() > 100 * 1024 * 1024 {
        return Err("导出文件超过 100 MB".into());
    }
    let bytes = bytes.clone();
    tauri::async_runtime::spawn_blocking(move || plan_export::save_file(window, bytes, extension))
        .await
        .map_err(|e| e.to_string())?
}
