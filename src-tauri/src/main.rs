#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod database;
mod scores;
mod spreadsheet;
mod seating;
mod plans;
mod plan_export;
mod plan_commands;
mod recovery;
mod ai;
mod ai_settings;
mod about;
#[cfg(test)]
mod ipc_tests;

use rusqlite::Connection;
use serde_json::{Map, Value};
use std::sync::{Mutex, MutexGuard, atomic::{AtomicBool, Ordering}};
use tauri::{Manager, State};

struct Database(Mutex<Connection>, AtomicBool);

impl Database {
    fn connection(&self) -> Result<MutexGuard<'_, Connection>, String> {
        let guard = self.0.lock().map_err(|_| "数据库锁不可用")?;
        if self.1.load(Ordering::Acquire) {
            return Err("恢复尚未完成，请在备份与恢复页面重试回退".into());
        }
        Ok(guard)
    }
}

#[tauri::command]
fn desktop_status(database: State<'_, Database>) -> &'static str {
    if database.1.load(Ordering::Acquire) { "恢复待处理，请打开备份与恢复" } else { "本地服务已连接" }
}

#[derive(serde::Serialize)]
struct DashboardClass {
    id: i64,
    name: String,
    students: i64,
    exams: i64,
    layouts: i64,
    max_seats: i64,
    rules: i64,
    plans: i64,
}

#[tauri::command]
fn dashboard_summary(database: State<'_, Database>) -> Result<Vec<DashboardClass>, String> {
    let db = database.connection()?;
    let mut query = db.prepare("SELECT c.id,c.name,
        (SELECT count(*) FROM seat_student WHERE class_id=c.id AND active=1),
        (SELECT count(*) FROM seat_exam WHERE class_id=c.id),
        (SELECT count(*) FROM seat_classroom room WHERE room.class_id=c.id AND room.active=1 AND
            EXISTS(SELECT 1 FROM seat_position WHERE classroom_id=room.id AND active=1 AND kind='seat')),
        (SELECT coalesce(max((SELECT count(*) FROM seat_position WHERE classroom_id=room.id AND active=1 AND
            kind='seat' AND row_number<=room.rows AND column_number<=room.columns)),0)
            FROM seat_classroom room WHERE room.class_id=c.id AND room.active=1),
        (SELECT count(*) FROM seat_rule WHERE class_id=c.id AND active=1 AND enabled=1) +
        (SELECT count(*) FROM seat_student_relation WHERE class_id=c.id AND active=1),
        (SELECT count(*) FROM seat_plan WHERE class_id=c.id)
        FROM seat_class c WHERE c.active=1 ORDER BY c.id").map_err(|error| error.to_string())?;
    let rows = query.query_map([], |row| Ok(DashboardClass {
        id: row.get(0)?, name: row.get(1)?, students: row.get(2)?, exams: row.get(3)?,
        layouts: row.get(4)?, max_seats: row.get(5)?, rules: row.get(6)?, plans: row.get(7)?,
    })).map_err(|error| error.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|error| error.to_string())
}

#[tauri::command]
fn list_records(database: State<'_, Database>, resource: String, class_id: Option<i64>) -> Result<Vec<Value>, String> {
    let connection = database.connection()?;
    database::list(&connection, &resource, class_id)
}

#[tauri::command]
fn save_record(database: State<'_, Database>, resource: String, data: Map<String, Value>, expected: Option<Map<String, Value>>) -> Result<i64, String> {
    let mut connection = database.connection()?;
    database::save_checked(&mut connection, &resource, data, expected)
}

#[tauri::command]
fn deactivate_records(database: State<'_, Database>, resource: String, class_id: Option<i64>, expected: Vec<Map<String, Value>>) -> Result<(), String> {
    let mut connection = database.connection()?;
    database::deactivate_records(&mut connection, &resource, class_id, expected)
}

#[tauri::command]
fn save_layout(
    database: State<'_, Database>,
    class_id: i64,
    classroom_id: i64,
    expected_rows: i64,
    expected_columns: i64,
    changes: Vec<database::PositionChange>,
) -> Result<(), String> {
    let mut connection = database.connection()?;
    database::save_layout(&mut connection, class_id, classroom_id, expected_rows, expected_columns, &changes)
}

#[tauri::command]
fn save_classroom_with_layout(database: State<'_, Database>, data: Map<String, Value>, expected: Option<Map<String, Value>>, changes: Vec<database::PositionChange>) -> Result<i64, String> {
    let mut connection = database.connection()?;
    database::save_classroom_with_layout(&mut connection, data, expected, &changes)
}

#[tauri::command]
fn copy_class(database: State<'_, Database>, options: database::CopyOptions) -> Result<i64, String> {
    let mut connection = database.connection()?;
    database::copy_class(&mut connection, options)
}

#[tauri::command]
fn list_exams(database: State<'_, Database>, class_id: i64) -> Result<Vec<scores::Exam>, String> {
    let connection = database.connection()?;
    scores::exams(&connection, class_id)
}

#[tauri::command]
fn save_exam(database: State<'_, Database>, exam: scores::Exam, expected: Option<scores::Exam>) -> Result<i64, String> {
    let mut connection = database.connection()?;
    scores::save_exam(&mut connection, exam, expected)
}

#[tauri::command]
fn list_scores(database: State<'_, Database>, exam_id: i64) -> Result<Vec<scores::ScoreRow>, String> {
    let connection = database.connection()?;
    scores::score_rows(&connection, exam_id)
}

#[tauri::command]
fn sync_score_levels(database: State<'_, Database>, exam_id: i64) -> Result<usize, String> {
    let mut connection = database.connection()?;
    scores::sync_levels(&mut connection, exam_id)
}

#[tauri::command]
fn save_scores(database: State<'_, Database>, exam_id: i64, input: Vec<scores::ScoreInput>, expected: Option<String>, remarks: Option<Vec<scores::ScoreRemark>>) -> Result<(), String> {
    let mut connection = database.connection()?;
    if let Some(remarks) = remarks { scores::save_score_changes(&mut connection, exam_id, input, remarks, expected) }
    else { scores::save_scores(&mut connection, exam_id, input, expected) }
}

#[tauri::command]
fn remove_score(database: State<'_, Database>, exam_id: i64, student_id: i64, expected: String) -> Result<(), String> {
    let mut connection = database.connection()?;
    scores::remove_score(&mut connection, exam_id, student_id, &expected)
}

#[tauri::command]
fn preview_scores(database: State<'_, Database>, exam_id: i64, bytes: Vec<u8>) -> Result<spreadsheet::ScorePreview, String> {
    let connection = database.connection()?;
    spreadsheet::score_preview(&connection, exam_id, bytes)
}

#[tauri::command]
fn score_template(database: State<'_, Database>, exam_id: i64) -> Result<Vec<u8>, String> {
    let connection = database.connection()?;
    spreadsheet::score_template(&connection, exam_id)
}

#[tauri::command]
fn export_scores(database: State<'_, Database>, exam_id: i64, by_student_no: bool) -> Result<Vec<u8>, String> {
    let connection = database.connection()?;
    spreadsheet::score_workbook(&connection, exam_id, true, by_student_no)
}

#[tauri::command]
fn export_records(database: State<'_, Database>, resource: String, class_id: Option<i64>, ids: Vec<i64>) -> Result<Vec<u8>, String> {
    let connection = database.connection()?;
    let ids: std::collections::HashSet<_> = ids.into_iter().collect();
    let mut records = database::list(&connection, &resource, class_id)?;
    records.retain(|item| item["id"].as_i64().is_some_and(|id| ids.contains(&id)));
    spreadsheet::records_workbook(&resource, &records)
}

#[tauri::command]
fn preview_students(database: State<'_, Database>, class_id: i64, bytes: Vec<u8>, update_existing: Option<bool>) -> Result<spreadsheet::StudentPreview, String> {
    let connection = database.connection()?;
    spreadsheet::student_preview(&connection, class_id, bytes, update_existing.unwrap_or(false))
}

#[tauri::command]
fn import_students(database: State<'_, Database>, class_id: i64, students: Vec<spreadsheet::StudentImport>, expected: String, update_existing: Option<bool>) -> Result<(), String> {
    let mut connection = database.connection()?;
    spreadsheet::save_students(&mut connection, class_id, students, expected, update_existing.unwrap_or(false))
}

#[tauri::command]
fn student_template() -> Result<Vec<u8>, String> { spreadsheet::student_template() }

#[tauri::command]
fn list_subject_templates(database: State<'_, Database>) -> Result<Vec<scores::SubjectTemplate>, String> {
    let connection = database.connection()?;
    scores::subject_templates(&connection)
}

#[tauri::command]
fn save_subject_template(database: State<'_, Database>, subject: scores::SubjectTemplate, expected: Option<scores::SubjectTemplate>) -> Result<(), String> {
    let mut connection = database.connection()?;
    scores::save_subject_template(&mut connection, subject, expected)
}

fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![desktop_status, dashboard_summary, list_records, save_record, deactivate_records, save_layout, save_classroom_with_layout, copy_class, list_exams, save_exam, list_scores, sync_score_levels, save_scores, remove_score, preview_scores, score_template, export_scores, export_records, preview_students, import_students, student_template, list_subject_templates, save_subject_template,
        plan_commands::inspect_plan_inputs, plan_commands::new_manual_plan, plan_commands::generate_plan, plan_commands::regenerate_plan, plan_commands::adjust_plan, plan_commands::list_plans, plan_commands::open_plan, plan_commands::save_plan, plan_commands::save_active_plan, plan_commands::activate_plan, plan_commands::copy_plan, plan_commands::plan_export_sheet, plan_commands::export_plan_workbook, plan_commands::export_plan_image,
        recovery::recovery_status, recovery::export_backup, recovery::inspect_backup, recovery::apply_backup, recovery::discard_backup, recovery::retry_rollback,
        ai::ai_settings, ai::save_ai_settings, ai::test_ai_connection, ai::list_ai_profiles, ai::save_ai_profile,
        ai::change_ai_profile, ai::test_ai_profile, ai::fetch_ai_models, ai::prepare_ai, ai::run_ai, ai::cancel_ai, ai::fallback_ai,
        about::about_info, about::check_app_update, about::download_update_asset])
}

fn main() {
    with_commands(tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            let storage = recovery::Storage::new(directory.clone())?;
            let (connection, blocked) = storage.open().map_err(std::io::Error::other)?;
            app.manage(Database(Mutex::new(connection), AtomicBool::new(blocked)));
            app.manage(storage);
            app.manage(ai::AiState::new(directory, app.config().identifier.clone()));
            Ok(())
        }))
        .run(tauri::generate_context!())
        .expect("运行班伴桌面应用失败");
}
