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
fn commands_accept_frontend_payloads_and_preserve_data_across_copy() {
    let mut connection = Connection::open_in_memory().unwrap();
    database::initialize(&mut connection).unwrap();
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default())))
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
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default())))
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
        let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default())))
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
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default())))
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
    let app = with_commands(mock_builder().manage(Database(Mutex::new(connection), Default::default())))
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
