use super::*;
use serde_json::json;
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY},
    webview::InvokeRequest,
    WebviewWindow, WebviewWindowBuilder,
};

fn call(window: &WebviewWindow<MockRuntime>, command: &str, body: Value) -> Result<Value, Value> {
    get_ipc_response(window, InvokeRequest {
        cmd: command.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: window.url().unwrap(),
        body: InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.into(),
    }).map(|body| body.deserialize::<Value>().unwrap())
}

#[test]
fn app_lock_credentials_and_settings_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app-lock.json");
    let builder = || with_commands(mock_builder().manage(app_lock::AppLock::load(directory.path().into())))
        .build(mock_context(noop_assets())).unwrap();
    let setup = {
        let app = builder();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        assert_eq!(call(&window, "app_lock_status", json!({})).unwrap()["enabled"], false);
        assert!(!path.exists());
        for password in ["短密码", "      ", "valid\tpassword"] {
            assert!(call(&window, "setup_app_lock", json!({"password":password})).is_err());
        }
        assert!(call(&window, "setup_app_lock", json!({"password":"fixture","idleTimeoutMinutes":6})).is_err());
        let response = call(&window, "setup_app_lock", json!({
            "password":" 多语言密码 ", "idleTimeoutMinutes":5
        })).unwrap();
        let on_disk: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 1);
        assert_eq!(on_disk["idle_timeout_minutes"], 5);
        for field in ["password_hash", "recovery_hash"] {
            assert!(on_disk[field].as_str().unwrap().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        }
        assert_ne!(on_disk["password_hash"], on_disk["recovery_hash"]);
        response
    };
    let code = setup["recovery_code"].as_str().unwrap();
    assert!(code.split('-').count() == 4 && code.split('-').all(|part| part.len() == 4));

    let app = builder();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let status = call(&window, "app_lock_status", json!({})).unwrap();
    assert_eq!(status["locked"], true);
    assert_eq!(status["idle_timeout_minutes"], 5);
    assert_eq!(status.as_object().unwrap().len(), 6);
    assert!(call(&window, "unlock_app", json!({"password":"多语言密码"})).is_err());
    assert_eq!(call(&window, "unlock_app", json!({"password":" 多语言密码 "})).unwrap()["locked"], false);

    assert!(call(&window, "change_app_lock_password", json!({"currentPassword":"incorrect","newPassword":"fixture-B"})).is_err());
    call(&window, "change_app_lock_password", json!({"currentPassword":" 多语言密码 ","newPassword":"fixture-B"})).unwrap();
    let status = call(&window, "update_app_lock_settings", json!({"idleTimeoutMinutes":30})).unwrap();
    assert_eq!(status["idle_timeout_minutes"], 30);
    let regenerated = call(&window, "regenerate_recovery_code", json!({"currentPassword":"fixture-B"})).unwrap();
    let replacement = regenerated["recovery_code"].as_str().unwrap();
    assert_ne!(replacement, code);
    call(&window, "lock_app", json!({})).unwrap();
    assert!(call(&window, "recover_app_lock", json!({"recoveryCode":code,"newPassword":"fixture-C"})).is_err());
    let normalized = replacement.to_ascii_lowercase().replace('-', " ");
    assert!(call(&window, "recover_app_lock", json!({"recoveryCode":normalized,"newPassword":"short"})).is_err());
    let recovered = call(&window, "recover_app_lock", json!({"recoveryCode":normalized,"newPassword":"fixture-C"})).unwrap();
    assert_ne!(replacement, recovered["recovery_code"]);
    assert_eq!(recovered["status"]["locked"], false);
    assert_eq!(recovered["status"]["idle_timeout_minutes"], 30);
    call(&window, "lock_app", json!({})).unwrap();
    assert!(call(&window, "recover_app_lock", json!({"recoveryCode":replacement,"newPassword":"fixture-D"})).is_err());
    assert!(call(&window, "unlock_app", json!({"password":"fixture-B"})).is_err());
    call(&window, "unlock_app", json!({"password":"fixture-C"})).unwrap();
    assert!(call(&window, "disable_app_lock", json!({"currentPassword":"fixture-B"})).is_err());
    assert_eq!(call(&window, "disable_app_lock", json!({"currentPassword":"fixture-C"})).unwrap()["enabled"], false);
    assert!(!path.exists());
    assert_eq!(call(&window, "lock_app", json!({})).unwrap_err(), "尚未开启应用锁");
}

#[test]
fn locked_dispatch_denies_everything_except_authentication_and_preserves_restore_block() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database(Mutex::new(Connection::open_in_memory().unwrap()), AtomicBool::new(true));
    let app = with_commands(mock_builder().manage(db).manage(app_lock::AppLock::load(directory.path().into())))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    call(&window, "setup_app_lock", json!({"password":"fixture-password"})).unwrap();
    assert_eq!(call(&window, "lock_app", json!({})).unwrap()["locked"], true);
    for command in [
        "future_command", "desktop_status", "dashboard_summary", "list_records", "save_record",
        "recovery_status", "apply_backup", "retry_rollback", "export_backup", "inspect_backup",
        "export_plan_workbook", "export_plan_image", "export_scores", "export_records", "download_update_asset", "run_ai",
        "lock_app", "setup_app_lock", "change_app_lock_password", "regenerate_recovery_code",
        "update_app_lock_settings", "disable_app_lock",
    ] {
        assert_eq!(call(&window, command, json!({})).unwrap_err(), app_lock::LOCKED, "{command}");
    }
    assert_eq!(call(&window, "app_lock_status", json!({})).unwrap()["locked"], true);
    let mut ran = false;
    assert_eq!(app_lock::with_unlocked(app.handle(), || { ran = true; Ok(()) }).unwrap_err(), app_lock::LOCKED);
    assert!(!ran);
    assert_eq!(call(&window, "unlock_app", json!({"password":"fixture-password"})).unwrap()["locked"], false);
    assert!(app.state::<Database>().1.load(std::sync::atomic::Ordering::Acquire));
    assert!(app_lock::ensure_unlocked(app.handle()).is_ok());
    assert!(call(&window, "dashboard_summary", json!({})).is_err());
}

#[test]
fn protected_ipc_rejects_reads_and_writes_when_app_lock_is_missing() {
    let mut connection = Connection::open_in_memory().unwrap();
    database::initialize(&mut connection).unwrap();
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default())))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    for (command, payload) in [
        ("dashboard_summary", json!({})),
        ("list_records", json!({"resource":"classes","classId":null})),
        ("save_record", json!({
            "resource":"classes", "expected":null,
            "data":{"name":"测试班","school_stage":"小学","grade":"一年级","term":"上学期","subjects":"语文","active":1}
        })),
    ] {
        assert_eq!(call(&window, command, payload).unwrap_err(), "应用锁暂不可用", "{command}");
    }
    let database = app.state::<Database>();
    assert!(database::list(&database.connection().unwrap(), "classes", None).unwrap().is_empty());
}

#[test]
fn export_commit_rechecks_the_lock_before_creating_or_replacing_a_file() {
    let directory = tempfile::tempdir().unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    database::initialize(&mut connection).unwrap();
    connection.execute(
        "INSERT INTO seat_class(id,name,school_stage,grade,term,subjects,active) VALUES(1,'测试班','初中','七年级','上学期','数学',1)",
        [],
    ).unwrap();
    connection.execute(
        "INSERT INTO seat_student(id,class_id,student_no,name,active) VALUES(1,1,'001','测试甲',1)",
        [],
    ).unwrap();
    let exam_id = scores::save_exam(&mut connection, scores::Exam {
        id: 0,
        class_id: 1,
        name: "期中".into(),
        exam_date: "2026-11-01".into(),
        remark: String::new(),
        rank_grading: true,
        is_current: true,
        thresholds: [90, 80, 70, 60],
        subjects: vec![scores::Subject {
            id: 0,
            name: "数学".into(),
            full_score: 10000,
            counts_toward_total: true,
        }],
    }, None).unwrap();
    let score_template = spreadsheet::score_template(&connection, exam_id).unwrap();
    let app = with_commands(mock_builder().manage(app_lock::AppLock::load(directory.path().into())))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    call(&window, "setup_app_lock", json!({"password":"fixture-password"})).unwrap();
    let new_file = directory.path().join("new.xlsx");
    let existing_file = directory.path().join("existing.xlsx");
    let original = b"original workbook fixture";
    let workbook = spreadsheet::records_workbook("classes", &[json!({"name":"测试班"})]).unwrap();
    std::fs::write(&existing_file, original).unwrap();

    // Simulate a lock after the save dialog's check and before the final rename.
    app_lock::ensure_unlocked(app.handle()).unwrap();
    call(&window, "lock_app", json!({})).unwrap();
    for bytes in [&workbook, &score_template] {
        for path in [&new_file, &existing_file] {
            assert_eq!(plan_export::persist_locked_bytes(app.handle(), path, bytes).unwrap_err(), app_lock::LOCKED);
        }
    }
    assert!(!new_file.exists());
    assert_eq!(std::fs::read(&existing_file).unwrap(), original);

    call(&window, "unlock_app", json!({"password":"fixture-password"})).unwrap();
    plan_export::persist_locked_bytes(app.handle(), &existing_file, &workbook).unwrap();
    assert_eq!(std::fs::read(&existing_file).unwrap(), workbook);
    plan_export::persist_locked_bytes(app.handle(), &new_file, &score_template).unwrap();
    assert_eq!(std::fs::read(&new_file).unwrap(), score_template);
}

#[test]
fn unreadable_or_invalid_app_lock_config_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app-lock.json");
    let valid = {
        let app = with_commands(mock_builder().manage(app_lock::AppLock::load(directory.path().into())))
            .build(mock_context(noop_assets())).unwrap();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        call(&window, "setup_app_lock", json!({"password":"fixture-password"})).unwrap();
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap()
    };
    let mut invalid_hash = valid.clone();
    invalid_hash["recovery_hash"] = json!("$argon2id$v=19$m=999999,t=2,p=1$not-a-valid-hash");
    let mut version = valid;
    version["version"] = json!(2);
    let verify = |directory: &std::path::Path| {
        let app = with_commands(mock_builder().manage(app_lock::AppLock::load(directory.into())))
            .build(mock_context(noop_assets())).unwrap();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let status = call(&window, "app_lock_status", json!({})).unwrap();
        for property in ["enabled", "locked", "corrupted"] {
            assert_eq!(status[property], true);
        }
        assert_eq!(call(&window, "about_info", json!({})).unwrap_err(), app_lock::LOCKED);
        for (command, args) in [
            ("unlock_app", json!({"password":"fixture-password"})),
            ("recover_app_lock", json!({"recoveryCode":"ABCD-ABCD-ABCD-ABCD","newPassword":"fixture-new"})),
        ] {
            assert!(call(&window, command, args).unwrap_err().as_str().unwrap().contains("app-lock.json"));
        }
    };
    for contents in ["{".to_string(), invalid_hash.to_string(), version.to_string()] {
        std::fs::write(&path, contents).unwrap();
        verify(directory.path());
    }
    let unreadable = tempfile::tempdir().unwrap();
    std::fs::create_dir(unreadable.path().join("app-lock.json")).unwrap();
    verify(unreadable.path());
}

#[test]
fn model_discovery_only_sends_credentials_for_the_selected_service() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn models_service(count: usize) -> (String, std::thread::JoinHandle<Vec<Option<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut received = Vec::new();
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut lines = BufReader::new(&stream).lines().map(Result::unwrap);
                assert!(lines.next().unwrap().starts_with("GET /v1/models "));
                let auth = lines.take_while(|line| !line.is_empty())
                    .filter_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("authorization").then(|| value.trim().to_owned())
                    }).last();
                received.push(auth);
                let body = r#"{"data":[{"id":"demo"}]}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            received
        });
        (endpoint, worker)
    }

    let storage = tempfile::tempdir().unwrap();
    let app = with_commands(mock_builder()
        .manage(ai::AiState::new(storage.path().into(), "test.endpoint".into()))
        .manage(app_lock::AppLock::disabled()))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let (a, received_a) = models_service(1);
    let (b, received_b) = models_service(2);
    let fetch = |endpoint: &str, key: Value| call(&window, "fetch_ai_models", json!({
        "profile":{"id":0,"name":"测试","provider":"CUSTOM","active":true,
            "settings":{"endpoint":endpoint,"model":"demo","timeout_seconds":5}},
        "key":key, "useSavedKey":false
    }));
    let key_a = json!({"endpoint":a,"value":"fixture-A"});
    assert_eq!(fetch(&a, key_a.clone()).unwrap(), json!(["demo"]));
    assert!(fetch(&b, key_a).is_err());
    assert_eq!(fetch(&b, Value::Null).unwrap(), json!(["demo"]));
    assert_eq!(fetch(&b, json!({"endpoint":b,"value":"fixture-B"})).unwrap(), json!(["demo"]));
    assert_eq!(received_a.join().unwrap(), vec![Some("Bearer fixture-A".into())]);
    assert_eq!(received_b.join().unwrap(), vec![None, Some("Bearer fixture-B".into())]);
}

#[test]
fn commands_accept_frontend_payloads_and_preserve_data_across_copy() {
    let mut connection = Connection::open_in_memory().unwrap();
    database::initialize(&mut connection).unwrap();
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default()))
        .manage(app_lock::AppLock::disabled()))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    assert_eq!(call(&window, "desktop_status", json!({})).unwrap(), "本地服务已连接");

    let class_id = call(&window, "save_record", json!({
        "resource":"classes", "expected":null,
        "data":{"name":"测试班","school_stage":"小学","grade":"一年级","term":"上学期","subjects":"语文","active":1}
    })).unwrap().as_i64().unwrap();
    let student_id = call(&window, "save_record", json!({
        "resource":"students", "expected":null,
        "data":{"class_id":class_id,"student_no":"001","name":"测试学生","gender":"","height":170,"eyesight":"","discipline":"","notes":"","active":1}
    })).unwrap().as_i64().unwrap();
    let second_student = call(&window, "save_record", json!({
        "resource":"students", "expected":null,
        "data":{"class_id":class_id,"student_no":"002","name":"另一位学生","gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":1}
    })).unwrap().as_i64().unwrap();
    let relation = json!({"class_id":class_id,"student_id":second_student,"other_student_id":student_id,"kind":"NOT_DESKMATE","active":1});
    call(&window, "save_record", json!({"resource":"relations","data":relation,"expected":null})).unwrap();
    assert!(call(&window, "save_record", json!({"resource":"relations","data":relation,"expected":null})).is_err());
    let classroom_id = call(&window, "save_record", json!({
        "resource":"classrooms", "expected":null,
        "data":{"class_id":class_id,"name":"测试教室","rows":2,"columns":2,"podium":"front","active":1}
    })).unwrap().as_i64().unwrap();
    call(&window, "save_layout", json!({
        "classId":class_id, "classroomId":classroom_id, "expectedRows":2, "expectedColumns":2,
        "changes":[
            {"row_number":1,"column_number":1,"expected":null,"kind":"seat","desk_group":"第一组","active":1},
            {"row_number":1,"column_number":2,"expected":null,"kind":"aisle","desk_group":"","active":1}
        ]
    })).unwrap();
    assert!(call(&window, "save_layout", json!({
        "classId":class_id, "classroomId":classroom_id, "expectedRows":2, "expectedColumns":2,
        "changes":[{"row_number":1,"column_number":1,"expected":null,"kind":"aisle","desk_group":"","active":1}]
    })).is_err());
    let front_rule = json!({"type":"FRONT_ROW","student_id":student_id,"max_row":1}).to_string();
    call(&window, "save_record", json!({
        "resource":"rules", "expected":null,
        "data":{"class_id":class_id,"name":"前排","kind":"HARD","weight":1,"config":front_rule,"active":1}
    })).unwrap();

    let students = call(&window, "list_records", json!({"resource":"students","classId":class_id})).unwrap();
    let original = students[0].clone();
    let mut updated = original.clone();
    updated["name"] = json!("更新学生");
    call(&window, "save_record", json!({"resource":"students","data":updated,"expected":original})).unwrap();
    assert!(call(&window, "save_record", json!({"resource":"students","data":updated,"expected":original})).is_err());

    let copied = call(&window, "copy_class", json!({
        "options":{"source_id":class_id,"name":"新学期","term":"下学期","students":true,"relations":true,"rules":true,"classrooms":true}
    })).unwrap().as_i64().unwrap();
    let copied_students = call(&window, "list_records", json!({"resource":"students","classId":copied})).unwrap();
    let copied_rules = call(&window, "list_records", json!({"resource":"rules","classId":copied})).unwrap();
    let copied_positions = call(&window, "list_records", json!({"resource":"positions","classId":copied})).unwrap();
    let copied_relations = call(&window, "list_records", json!({"resource":"relations","classId":copied})).unwrap();
    let config: Value = serde_json::from_str(copied_rules[0]["config"].as_str().unwrap()).unwrap();
    assert_eq!(copied_students[0]["name"], "更新学生");
    assert_ne!(copied_students[0]["id"], student_id);
    assert_eq!(config["student_id"], copied_students[0]["id"]);
    assert_eq!(copied_positions.as_array().unwrap().len(), 2);
    assert_eq!(copied_relations.as_array().unwrap().len(), 1);
    assert_eq!(copied_relations[0]["student_id"], copied_students[0]["id"]);
    assert_eq!(copied_relations[0]["other_student_id"], copied_students[1]["id"]);
}

#[test]
fn ipc_can_disable_legacy_rule_without_changing_its_config() {
    let mut connection = Connection::open_in_memory().unwrap();
    database::initialize(&mut connection).unwrap();
    connection.execute(
        "INSERT INTO seat_class(id,name,school_stage,grade,term,subjects,active) VALUES(1,'测试班','','','','',1)", []
    ).unwrap();
    let config = r#"  {"type":"FRONT_ROW"}  "#;
    connection.execute(
        "INSERT INTO seat_rule(class_id,name,kind,weight,config,active) VALUES(1,'旧规则','HARD',1,?1,1)", [config]
    ).unwrap();
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default()))
        .manage(app_lock::AppLock::disabled()))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let original = call(&window, "list_records", json!({"resource":"rules","classId":1})).unwrap()[0].clone();
    let mut disabled = original.clone();
    disabled["active"] = json!(0);
    call(&window, "save_record", json!({"resource":"rules","data":disabled,"expected":original})).unwrap();
    let stored = call(&window, "list_records", json!({"resource":"rules","classId":1})).unwrap()[0].clone();
    assert_eq!(stored["config"], config);
    assert_eq!(stored["active"], 0);
    let mut enabled = stored.clone();
    enabled["active"] = json!(1);
    assert!(call(&window, "save_record", json!({"resource":"rules","data":enabled,"expected":stored})).is_err());
}

#[test]
fn ipc_persists_complete_class_data_across_reopening() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target")
        .join(format!("phase2-ipc-reopen-{}-{stamp}.db", std::process::id()));
    let resources = ["students", "classrooms", "positions", "rules", "relations"];
    let (class_id, before) = {
        let mut connection = Connection::open(&path).unwrap();
        database::initialize(&mut connection).unwrap();
        let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default()))
            .manage(app_lock::AppLock::disabled()))
            .build(mock_context(noop_assets())).unwrap();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let class_id = call(&window, "save_record", json!({
            "resource":"classes","expected":null,
            "data":{"name":"测试班","school_stage":"小学","grade":"一年级","term":"上学期","subjects":"语文","active":1}
        })).unwrap().as_i64().unwrap();
        let student = |number| call(&window, "save_record", json!({
            "resource":"students","expected":null,
            "data":{"class_id":class_id,"student_no":number,"name":"测试学生","gender":"","height":170,"eyesight":"","discipline":"","notes":"","active":1}
        })).unwrap().as_i64().unwrap();
        let first = student("001");
        let second = student("002");
        call(&window, "save_record", json!({
            "resource":"relations","expected":null,
            "data":{"class_id":class_id,"student_id":first,"other_student_id":second,"kind":"NOT_DESKMATE","active":1}
        })).unwrap();
        call(&window, "save_record", json!({
            "resource":"rules","expected":null,
            "data":{"class_id":class_id,"name":"前排","kind":"HARD","weight":1,
                "config":json!({"type":"FRONT_ROW","student_id":first,"max_row":1}).to_string(),"active":1}
        })).unwrap();
        let classroom = call(&window, "save_record", json!({
            "resource":"classrooms","expected":null,
            "data":{"class_id":class_id,"name":"教室","rows":2,"columns":2,"podium":"front","active":1}
        })).unwrap().as_i64().unwrap();
        call(&window, "save_layout", json!({
            "classId":class_id,"classroomId":classroom,"expectedRows":2,"expectedColumns":2,
            "changes":[
                {"row_number":1,"column_number":1,"expected":null,"kind":"seat","desk_group":"第一组","active":1},
                {"row_number":1,"column_number":2,"expected":null,"kind":"aisle","desk_group":"","active":1}
            ]
        })).unwrap();
        let before = resources.iter().map(|resource|
            call(&window, "list_records", json!({"resource":resource,"classId":class_id})).unwrap()
        ).collect::<Vec<_>>();
        (class_id, before)
    };
    let mut connection = Connection::open(&path).unwrap();
    database::initialize(&mut connection).unwrap();
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default()))
        .manage(app_lock::AppLock::disabled()))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    for (index, resource) in resources.iter().enumerate() {
        assert_eq!(call(&window, "list_records", json!({"resource":resource,"classId":class_id})).unwrap(), before[index]);
    }
    let copied = call(&window, "copy_class", json!({
        "options":{"source_id":class_id,"name":"新学期","term":"下学期","students":true,"relations":true,"rules":true,"classrooms":true}
    })).unwrap().as_i64().unwrap();
    let students = call(&window, "list_records", json!({"resource":"students","classId":copied})).unwrap();
    let rules = call(&window, "list_records", json!({"resource":"rules","classId":copied})).unwrap();
    let positions = call(&window, "list_records", json!({"resource":"positions","classId":copied})).unwrap();
    let config: Value = serde_json::from_str(rules[0]["config"].as_str().unwrap()).unwrap();
    assert_eq!(config["student_id"], students[0]["id"]);
    assert_eq!(positions[0]["desk_group"], "第一组");
    assert_eq!(positions[1]["kind"], "aisle");
}

#[test]
fn ipc_rule_and_relation_lifecycle_rejects_conflicts_and_cross_class_links() {
    let mut connection = Connection::open_in_memory().unwrap();
    database::initialize(&mut connection).unwrap();
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default()))
        .manage(app_lock::AppLock::disabled()))
        .build(mock_context(noop_assets())).unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let new_class = |name| call(&window, "save_record", json!({
        "resource":"classes","expected":null,
        "data":{"name":name,"school_stage":"","grade":"","term":"","subjects":"","active":1}
    })).unwrap().as_i64().unwrap();
    let class_id = new_class("一班");
    let other_class = new_class("二班");
    let new_student = |class_id, number| call(&window, "save_record", json!({
        "resource":"students","expected":null,
        "data":{"class_id":class_id,"student_no":number,"name":"测试学生","gender":"","height":null,"eyesight":"","discipline":"","notes":"","active":1}
    })).unwrap().as_i64().unwrap();
    let first = new_student(class_id, "001");
    let second = new_student(class_id, "002");
    let outsider = new_student(other_class, "001");
    let relation = json!({"class_id":class_id,"student_id":second,"other_student_id":first,"kind":"NOT_DESKMATE","active":1});
    call(&window, "save_record", json!({"resource":"relations","expected":null,"data":relation})).unwrap();
    let original = call(&window, "list_records", json!({"resource":"relations","classId":class_id})).unwrap()[0].clone();
    assert_eq!(original["student_id"], first);
    assert_eq!(original["other_student_id"], second);
    let mut edited = original.clone();
    edited["kind"] = json!("NOT_ADJACENT");
    edited["active"] = json!(0);
    call(&window, "save_record", json!({"resource":"relations","expected":original,"data":edited})).unwrap();
    let disabled = call(&window, "list_records", json!({"resource":"relations","classId":class_id})).unwrap()[0].clone();
    assert_eq!(disabled["kind"], "NOT_ADJACENT");
    assert_eq!(disabled["active"], 0);
    let mut invalid = disabled.clone();
    invalid["student_id"] = json!(outsider);
    assert!(call(&window, "save_record", json!({"resource":"relations","expected":disabled,"data":invalid})).is_err());
    let mut enabled = disabled.clone();
    enabled["active"] = json!(1);
    call(&window, "save_record", json!({"resource":"relations","expected":disabled,"data":enabled})).unwrap();
    assert_eq!(call(&window, "list_records", json!({"resource":"relations","classId":class_id})).unwrap()[0]["active"], 1);

    let config = json!({"type":"VISION_FRONT","extra":"保留参数"}).to_string();
    call(&window, "save_record", json!({
        "resource":"rules","expected":null,
        "data":{"class_id":class_id,"name":"视力","kind":"SOFT","weight":10,"config":config,"active":1}
    })).unwrap();
    let original_rule = call(&window, "list_records", json!({"resource":"rules","classId":class_id})).unwrap()[0].clone();
    let mut edited_rule = original_rule.clone();
    edited_rule["weight"] = json!(25);
    edited_rule["active"] = json!(0);
    call(&window, "save_record", json!({"resource":"rules","expected":original_rule,"data":edited_rule})).unwrap();
    let disabled_rule = call(&window, "list_records", json!({"resource":"rules","classId":class_id})).unwrap()[0].clone();
    assert_eq!(disabled_rule["weight"], 25);
    assert_eq!(disabled_rule["config"], config);
    let mut enabled_rule = disabled_rule.clone();
    enabled_rule["active"] = json!(1);
    call(&window, "save_record", json!({"resource":"rules","expected":disabled_rule,"data":enabled_rule})).unwrap();
    assert_eq!(call(&window, "list_records", json!({"resource":"rules","classId":class_id})).unwrap()[0]["active"], 1);
}
