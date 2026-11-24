use crate::{database, plans, seating, Database};
use rusqlite::{
    backup::{Backup, StepResult},
    Connection, OpenFlags,
};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tempfile::{NamedTempFile, TempPath};
use uuid::Uuid;

const PRODUCT_ID: i64 = 0x434D4852;
const EXTENSION: &str = "classmatehub-backup";
const POINT: &str = "before-restore.classmatehub-backup";
const PENDING: &str = "restore.pending";

pub struct Storage {
    directory: PathBuf,
    _instance: File,
    staged: Mutex<Option<(String, TempPath)>>,
    startup_notice: Mutex<Option<String>>,
}

#[derive(Serialize)]
pub struct Status {
    pending: bool,
    recovery_point: Option<String>,
    notice: Option<String>,
}

#[derive(Serialize)]
pub struct Summary {
    token: String,
    source_name: String,
    version: i64,
    incoming: Counts,
    current: Counts,
}

#[derive(Serialize)]
struct Counts {
    classes: i64,
    students: i64,
    exams: i64,
    plans: i64,
}

#[derive(Serialize)]
pub struct Outcome {
    status: &'static str,
    message: String,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("无效的目标目录"))?;
    let mut file = NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

impl Storage {
    pub fn data_dir(&self) -> &Path {
        &self.directory
    }

    pub fn new(directory: PathBuf) -> io::Result<Self> {
        let instance = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("instance.lock"))?;
        instance
            .try_lock()
            .map_err(|_| io::Error::other("同一数据目录已有班伴实例运行"))?;
        Ok(Self {
            directory,
            _instance: instance,
            staged: Mutex::new(None),
            startup_notice: Mutex::new(None),
        })
    }

    fn db_path(&self) -> PathBuf {
        self.directory.join("classmate_hub.db")
    }
    fn point(&self) -> PathBuf {
        self.directory.join(POINT)
    }
    fn marker(&self) -> PathBuf {
        self.directory.join(PENDING)
    }

    pub fn open(&self) -> Result<(Connection, bool), String> {
        if self.marker().try_exists().map_err(error)? {
            return match self.rollback() {
                Ok(connection) => {
                    *self.startup_notice.lock().map_err(error)? =
                        Some("检测到上次恢复中断，已回退到恢复前的数据".into());
                    Ok((connection, false))
                }
                Err(reason) => {
                    *self.startup_notice.lock().map_err(error)? =
                        Some(format!("恢复中断且回退未完成：{reason}"));
                    Ok((Connection::open_in_memory().map_err(error)?, true))
                }
            };
        }
        let mut connection = Connection::open(self.db_path()).map_err(error)?;
        database::initialize(&mut connection)?;
        Ok((connection, false))
    }

    fn rollback(&self) -> Result<Connection, String> {
        let source = open_readonly(&self.point())?;
        validate(&source, database::VERSION, true)?;
        let mut target = Connection::open(self.db_path()).map_err(error)?;
        copy_database(&source, &mut target)?;
        drop(target);
        let connection = self.verify_live()?;
        fs::remove_file(self.marker()).map_err(error)?;
        Ok(connection)
    }

    fn verify_live(&self) -> Result<Connection, String> {
        let mut connection =
            Connection::open_with_flags(self.db_path(), OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(error)?;
        validate(&connection, database::VERSION, true)?;
        database::initialize(&mut connection)?;
        Ok(connection)
    }

    fn inspect(&self, database: &Database, path: &Path) -> Result<Summary, String> {
        *self.staged.lock().map_err(error)? = None;
        let metadata = fs::metadata(path).map_err(error)?;
        if !metadata.is_file() {
            return Err("请选择可读取的备份文件".into());
        }
        let source = open_readonly(path)?;
        let version = version(&source).map_err(|_| "备份文件损坏或无法识别")?;
        if !(1..=database::VERSION).contains(&version) {
            return Err("备份版本无法识别或高于当前应用".into());
        }
        let product: i64 = source
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(|_| "无法识别备份的产品标识")?;
        if product != PRODUCT_ID {
            return Err("请选择班伴生成的备份文件".into());
        }
        let temp = NamedTempFile::new_in(&self.directory)
            .map_err(error)?
            .into_temp_path();
        let mut staged = Connection::open(&temp).map_err(error)?;
        copy_database(&source, &mut staged)?;
        validate(&staged, version, true)?;
        database::initialize(&mut staged)?;
        validate(&staged, database::VERSION, true)?;
        let incoming = counts(&staged)?;
        let current = counts(&*database.connection()?)?;
        drop(staged);
        let token = Uuid::new_v4().to_string();
        *self.staged.lock().map_err(error)? = Some((token.clone(), temp));
        Ok(Summary {
            token,
            source_name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            version,
            incoming,
            current,
        })
    }

    fn restore(&self, database: &Database, token: &str) -> Result<Outcome, String> {
        let path = {
            let mut staged = self.staged.lock().map_err(error)?;
            if !staged.as_ref().is_some_and(|(id, _)| id == token) {
                return Err("预检已失效，请重新选择备份".into());
            }
            staged.take().unwrap().1
        };
        self.restore_file(database, &path, || self.verify_live())
    }

    fn restore_file(
        &self,
        database: &Database,
        path: &Path,
        verify: impl FnOnce() -> Result<Connection, String>,
    ) -> Result<Outcome, String> {
        let source = open_readonly(path)?;
        validate(&source, database::VERSION, true)?;
        let mut target = database.connection()?;
        write_backup(&target, &self.point())?;
        write_atomic(&self.marker(), b"pending\n").map_err(error)?;
        database.1.store(true, Ordering::Release);
        let result = copy_database(&source, &mut target)
            .and_then(|_| verify())
            .and_then(|new| {
                fs::remove_file(self.marker()).map_err(error)?;
                Ok(new)
            });
        match result {
            Ok(connection) => {
                *target = connection;
                database.1.store(false, Ordering::Release);
                Ok(Outcome {
                    status: "restored",
                    message: "恢复完成，已加载备份中的数据".into(),
                })
            }
            Err(reason) => match self.rollback() {
                Ok(connection) => {
                    *target = connection;
                    database.1.store(false, Ordering::Release);
                    Ok(Outcome {
                        status: "rolled_back",
                        message: format!("恢复失败，已回退到恢复前的数据：{reason}"),
                    })
                }
                Err(_) => Ok(Outcome {
                    status: "blocked",
                    message: format!(
                        "恢复失败且回退未完成，已暂停业务访问。恢复点：{}",
                        self.point().display()
                    ),
                }),
            },
        }
    }
}

fn open_readonly(path: &Path) -> Result<Connection, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(error)?;
    db.pragma_update(None, "trusted_schema", false)
        .map_err(error)?;
    Ok(db)
}

fn copy_database(source: &Connection, target: &mut Connection) -> Result<(), String> {
    let backup = Backup::new(source, target).map_err(error)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(45) {
        match backup.step(256).map_err(error)? {
            StepResult::Done => return Ok(()),
            StepResult::Busy | StepResult::Locked => std::thread::sleep(Duration::from_millis(25)),
            _ => {}
        }
    }
    Err("SQLite 复制超时".into())
}

fn write_backup(source: &Connection, path: &Path) -> Result<(), String> {
    let directory = path.parent().ok_or("保存路径无效")?;
    let file = NamedTempFile::new_in(directory).map_err(error)?;
    let mut snapshot = Connection::open(file.path()).map_err(error)?;
    copy_database(source, &mut snapshot)?;
    snapshot
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(error)?;
    snapshot
        .pragma_update(None, "application_id", PRODUCT_ID)
        .map_err(error)?;
    validate(&snapshot, database::VERSION, true)?;
    snapshot.close().map_err(|(_, e)| error(e))?;
    file.as_file().sync_all().map_err(error)?;
    file.persist(path).map_err(error)?;
    Ok(())
}

fn version(db: &Connection) -> Result<i64, String> {
    db.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(error)
}

fn schema(db: &Connection) -> Result<Vec<(String, String, String)>, String> {
    let mut stmt = db
        .prepare("SELECT type,name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type,name")
        .map_err(error)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, String>(2)?.replace("\r\n", "\n"),
            ))
        })
        .map_err(error)?;
    rows.collect::<Result<_, _>>().map_err(error)
}

fn validate(db: &Connection, expected: i64, check_id: bool) -> Result<(), String> {
    let id: i64 = db
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(error)?;
    if (check_id && id != PRODUCT_ID) || version(db)? != expected {
        return Err("产品标识或备份版本不匹配".into());
    }
    if schema(db)? != schema(&database::schema_at(expected).map_err(error)?)? {
        return Err("备份的表结构不符合该版本".into());
    }
    let mut check = db.prepare("PRAGMA integrity_check").map_err(error)?;
    let mut results = check.query([]).map_err(error)?;
    if results
        .next()
        .map_err(error)?
        .map(|r| r.get::<_, String>(0))
        .transpose()
        .map_err(error)?
        .as_deref()
        != Some("ok")
        || results.next().map_err(error)?.is_some()
        || db
            .prepare("PRAGMA foreign_key_check")
            .map_err(error)?
            .exists([])
            .map_err(error)?
    {
        return Err("备份未通过数据库完整性或外键检查".into());
    }
    if expected == database::VERSION {
        let mut stmt = db
            .prepare("SELECT id,class_id FROM seat_plan")
            .map_err(error)?;
        let records = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .map_err(error)?;
        for row in records {
            let (plan_id, class_id) = row.map_err(error)?;
            let plan = plans::read(db, plan_id).map_err(|_| "备份中有无法读取的历史方案")?;
            if class_id != plan.snapshot.class_id
                || (plan.confirmed && plan.assignments.is_empty())
                || seating::validate_placements(&plan.snapshot, &plan.assignments, false).is_err()
            {
                return Err("备份中有不完整或违反硬约束的历史方案".into());
            }
        }
    }
    Ok(())
}

fn counts(db: &Connection) -> Result<Counts, String> {
    db.query_row("SELECT (SELECT count(*) FROM seat_class),(SELECT count(*) FROM seat_student),(SELECT count(*) FROM seat_exam),(SELECT count(*) FROM seat_plan)", [],
        |row| Ok(Counts {classes:row.get(0)?,students:row.get(1)?,exams:row.get(2)?,plans:row.get(3)?})).map_err(error)
}

#[tauri::command]
pub fn recovery_status(
    storage: State<'_, Storage>,
    database: State<'_, Database>,
) -> Result<Status, String> {
    let point = storage.point();
    Ok(Status {
        pending: database.1.load(Ordering::Acquire),
        recovery_point: point
            .try_exists()
            .map_err(error)?
            .then(|| point.display().to_string()),
        notice: storage.startup_notice.lock().map_err(error)?.take(),
    })
}

#[tauri::command]
pub async fn export_backup<R: tauri::Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("导出班伴备份")
            .set_file_name(format!("班伴备份.{EXTENSION}"))
            .add_filter("班伴备份", &[EXTENSION])
            .save_file()
        else {
            return Ok(false);
        };
        let storage = app.state::<Storage>();
        let parent = path
            .parent()
            .ok_or("保存路径无效")?
            .canonicalize()
            .map_err(error)?;
        if parent.starts_with(storage.directory.canonicalize().map_err(error)?)
            || path.extension().and_then(|e| e.to_str()) != Some(EXTENSION)
        {
            return Err("请选择应用数据目录之外的班伴备份文件".into());
        }
        write_backup(&*app.state::<Database>().connection()?, &path)?;
        Ok(true)
    })
    .await
    .map_err(error)?
}

#[tauri::command]
pub async fn inspect_backup<R: tauri::Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
) -> Result<Option<Summary>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("选择班伴备份")
            .add_filter("班伴备份", &[EXTENSION])
            .pick_file()
        else {
            return Ok(None);
        };
        app.state::<Storage>()
            .inspect(&app.state::<Database>(), &path)
            .map(Some)
    })
    .await
    .map_err(error)?
}

#[tauri::command]
pub async fn apply_backup<R: tauri::Runtime>(
    app: AppHandle<R>,
    token: String,
) -> Result<Outcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>()
            .restore(&app.state::<Database>(), &token)
    })
    .await
    .map_err(error)?
}

#[tauri::command]
pub fn discard_backup(storage: State<'_, Storage>, token: String) -> Result<(), String> {
    let mut staged = storage.staged.lock().map_err(error)?;
    if staged.as_ref().is_some_and(|(id, _)| id == &token) {
        *staged = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn retry_rollback<R: tauri::Runtime>(app: AppHandle<R>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let database = app.state::<Database>();
        let mut connection = database.0.lock().map_err(error)?;
        if !database.1.load(Ordering::Acquire) {
            return Err("当前没有中断的恢复".into());
        }
        *connection = app.state::<Storage>().rollback()?;
        database.1.store(false, Ordering::Release);
        Ok(())
    })
    .await
    .map_err(error)?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_name(db: &Connection) -> String {
        db.query_row("SELECT name FROM seat_class", [], |r| r.get(0))
            .unwrap()
    }

    fn add_class(db: &Connection, name: &str) {
        db.execute("INSERT INTO seat_class(name,school_stage,grade,term) VALUES(?,'初中','七年级','测试学期')",[name]).unwrap();
    }

    #[test]
    fn restores_exam_eligibility_student_levels_and_saved_desks_together() {
        use crate::scores::{self, ScoreInput, ScoreRemark};
        let temporary = tempfile::tempdir().unwrap();
        let storage = Storage::new(temporary.path().to_path_buf()).unwrap();
        let (mut live, _) = storage.open().unwrap();
        add_class(&live, "合成测试班");
        live.execute_batch("INSERT INTO seat_student(id,class_id,student_no,name) VALUES(1,1,'01','甲'),(2,1,'02','乙');
            INSERT INTO seat_exam(id,class_id,name,rank_grading) VALUES(1,1,'测试',1);
            INSERT INTO seat_exam_subject(id,exam_id,name,full_score) VALUES(1,1,'数学',10000);
            INSERT INTO seat_classroom(id,class_id,name,rows,columns,podium) VALUES(1,1,'教室',1,4,'front');
            INSERT INTO seat_position(class_id,classroom_id,row_number,column_number,kind)
            VALUES(1,1,1,1,'seat'),(1,1,1,2,'aisle'),(1,1,1,3,'seat'),(1,1,1,4,'seat');
            INSERT INTO seat_student_relation(class_id,student_id,other_student_id,kind) VALUES(1,1,2,'NOT_DESKMATE')").unwrap();
        let zero = ScoreInput { student_id: 1, subject_id: 1, value: "0".into() };
        let note = ScoreRemark { student_id: 2, remark: "待补录".into() };
        scores::save_score_changes(&mut live, 1, vec![zero], vec![note], None).unwrap();
        assert_eq!(scores::sync_levels(&mut live, 1).unwrap(), 1);
        let snapshot = plans::snapshot(&live, 1, 1, Some(1)).unwrap();
        let draft = plans::generate(snapshot, 41).unwrap();
        let saved = plans::save(&mut live, draft, true).unwrap();
        let backup = temporary.path().join("exam.classmatehub-backup");
        write_backup(&live, &backup).unwrap();
        scores::save_scores(&mut live, 1, vec![ScoreInput { student_id: 2, subject_id: 1, value: "95".into() }], None).unwrap();
        let database = Database(Mutex::new(live), Default::default());
        let summary = storage.inspect(&database, &backup).unwrap();
        assert_eq!(storage.restore(&database, &summary.token).unwrap().status, "restored");
        let live = database.connection().unwrap();
        let rows = scores::score_rows(&live, 1).unwrap();
        assert_eq!((rows[0].total_score, rows[0].total_rank), (Some(0), Some(1)));
        assert_eq!((rows[1].total_rank, rows[1].remark.as_str()), (None, "待补录"));
        let students = database::list(&live, "students", Some(1)).unwrap();
        assert_eq!(students[0]["score_level"], "A");
        assert_eq!(students[1]["score_level"], "");
        let plan = plans::read(&live, saved.id.unwrap()).unwrap();
        assert_eq!(plan.snapshot, saved.snapshot);
        assert_eq!(plan.assignments, saved.assignments);
    }

    #[test]
    fn v4_backup_is_upgraded_in_staging_and_keeps_business_records() {
        let temporary = tempfile::tempdir().unwrap();
        let storage = Storage::new(temporary.path().to_path_buf()).unwrap();
        let (live, _) = storage.open().unwrap();
        add_class(&live, "恢复前");
        let db = Database(Mutex::new(live), Default::default());
        let old = database::schema_at(4).unwrap();
        old.pragma_update(None, "user_version", 4).unwrap();
        add_class(&old, "旧版班级");
        old.execute_batch("INSERT INTO seat_classroom(id,class_id,name,rows,columns,podium) VALUES(1,1,'旧布局',2,2,'front');
            INSERT INTO seat_position(class_id,classroom_id,row_number,column_number,kind,desk_group) VALUES(1,1,1,1,'seat','旧分组');").unwrap();
        let backup = temporary.path().join("v4.classmatehub-backup");
        let mut backup_db = Connection::open(&backup).unwrap();
        copy_database(&old, &mut backup_db).unwrap();
        backup_db.pragma_update(None, "application_id", PRODUCT_ID).unwrap();
        drop(backup_db);
        let summary = storage.inspect(&db, &backup).unwrap();
        assert_eq!((summary.version, summary.incoming.classes), (4, 1));
        assert_eq!(storage.restore(&db, &summary.token).unwrap().status, "restored");
        let restored = db.connection().unwrap();
        assert_eq!(version(&restored).unwrap(), database::VERSION);
        assert_eq!(class_name(&restored), "旧版班级");
        assert_eq!(database::list(&restored, "positions", Some(1)).unwrap()[0]["desk_group"], "旧分组");
        assert_eq!(version(&open_readonly(&backup).unwrap()).unwrap(), 4);
    }

    #[test]
    fn restore_rechecks_and_rolls_back_after_the_new_database_was_committed() {
        let temporary = tempfile::tempdir().unwrap();
        let storage = Storage::new(temporary.path().to_path_buf()).unwrap();
        assert!(Storage::new(temporary.path().to_path_buf()).is_err());
        let (live, blocked) = storage.open().unwrap();
        assert!(!blocked);
        add_class(&live, "恢复前班级");
        let db = Database(Mutex::new(live), Default::default());
        let mut incoming = Connection::open_in_memory().unwrap();
        database::initialize(&mut incoming).unwrap();
        add_class(&incoming, "备份中的班级");
        let incoming_file = temporary.path().join("incoming.classmatehub-backup");
        write_backup(&incoming, &incoming_file).unwrap();
        let copy_source = temporary.path().join("old.classmatehub-backup");
        let file = Connection::open(&copy_source).unwrap();
        file.execute_batch(include_str!("../migrations/001_init.sql"))
            .unwrap();
        file.pragma_update(None, "user_version", 1).unwrap();
        file.pragma_update(None, "application_id", PRODUCT_ID)
            .unwrap();
        add_class(&file, "旧版班级");
        drop(file);
        let old_preview = storage.inspect(&db, &copy_source).unwrap();
        assert_eq!(old_preview.version, 1);
        assert_eq!(version(&open_readonly(&copy_source).unwrap()).unwrap(), 1);
        let corrupt = temporary.path().join("bad.classmatehub-backup");
        fs::write(&corrupt, b"bad sqlite").unwrap();
        assert!(storage.inspect(&db, &corrupt).is_err());
        assert!(storage.restore(&db, &old_preview.token).is_err());
        assert_eq!(class_name(&db.connection().unwrap()), "恢复前班级");
        let too_new =
            Connection::open(&corrupt.with_file_name("future.classmatehub-backup")).unwrap();
        too_new.pragma_update(None, "user_version", 99).unwrap();
        drop(too_new);
        assert!(storage
            .inspect(&db, &corrupt.with_file_name("future.classmatehub-backup"))
            .is_err());
        let outcome = storage
            .restore_file(&db, &incoming_file, || {
                assert_eq!(class_name(&storage.verify_live().unwrap()), "备份中的班级");
                Err("模拟提交后的校验失败".into())
            })
            .unwrap();
        assert_eq!(outcome.status, "rolled_back");
        assert_eq!(class_name(&db.connection().unwrap()), "恢复前班级");
        assert!(!storage.marker().exists());
        let token = storage.inspect(&db, &incoming_file).unwrap().token;
        assert_eq!(storage.restore(&db, &token).unwrap().status, "restored");
        assert_eq!(class_name(&db.connection().unwrap()), "备份中的班级");
        assert_eq!(
            class_name(&open_readonly(&storage.point()).unwrap()),
            "恢复前班级"
        );
        assert!(storage.restore(&db, &token).is_err());
    }

    #[test]
    fn startup_recovers_pending_restore_and_preserves_failed_recovery_point() {
        let temporary = tempfile::tempdir().unwrap();
        let storage = Storage::new(temporary.path().to_path_buf()).unwrap();
        let (live, _) = storage.open().unwrap();
        add_class(&live, "原数据");
        write_backup(&live, &storage.point()).unwrap();
        live.execute("UPDATE seat_class SET name='中断时的新数据'", [])
            .unwrap();
        drop(live);
        write_atomic(&storage.marker(), b"pending\n").unwrap();
        let (live, blocked) = storage.open().unwrap();
        assert!(!blocked && !storage.marker().exists());
        assert_eq!(class_name(&live), "原数据");
        drop(live);
        write_atomic(&storage.marker(), b"pending\n").unwrap();
        fs::write(storage.point(), b"simulated unreadable recovery point").unwrap();
        let (live, blocked) = storage.open().unwrap();
        let database = Database(Mutex::new(live), blocked.into());
        assert!(database.connection().is_err());
        assert!(storage.marker().exists() && storage.point().exists());
    }
}
