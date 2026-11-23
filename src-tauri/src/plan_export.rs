use crate::{
    plans::Plan,
    seating::{CellKind, Rating},
};
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook};
use serde::Serialize;
use std::{collections::HashMap, io::Write};
use tauri::WebviewWindow;

#[derive(Serialize)]
pub struct Sheet {
    pub class_name: String,
    pub rows: u16,
    pub columns: u16,
    pub podium_front: bool,
    pub podium: &'static str,
    pub cells: Vec<Cell>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratings: Option<Vec<Rating>>,
}

#[derive(Serialize)]
pub struct Cell {
    pub row: u16,
    pub column: u16,
    pub kind: CellKind,
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
}

pub fn project(plan: &Plan, teacher: bool) -> Sheet {
    let students: HashMap<_, _> = plan.snapshot.students.iter().map(|s| (s.id, s)).collect();
    let assignments: HashMap<_, _> = plan
        .assignments
        .iter()
        .map(|a| (a.position_id, a))
        .collect();
    let cells = plan
        .snapshot
        .positions
        .iter()
        .map(|p| {
            let assignment = assignments.get(&p.id);
            let student = assignment.and_then(|a| students.get(&a.student_id));
            Cell {
                row: p.row,
                column: p.column,
                kind: p.kind,
                name: student.map(|s| s.name.clone()),
                number: student.filter(|_| teacher).map(|s| s.number.clone()),
                locked: assignment.filter(|_| teacher).map(|a| a.locked),
            }
        })
        .collect();
    Sheet {
        class_name: plan.snapshot.class_name.clone(),
        rows: plan.snapshot.rows,
        columns: plan.snapshot.columns,
        podium_front: plan.snapshot.podium_front,
        podium: plan.snapshot.podium(),
        cells,
        ratings: teacher.then(|| plan.ratings.clone()),
    }
}

pub fn workbook(data: &Sheet) -> Result<Vec<u8>, String> {
    let mut workbook = Workbook::new();
    let grid = workbook.add_worksheet();
    grid.set_name("座位表").map_err(|e| e.to_string())?;
    let title = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_text_wrap();
    let cell = Format::new()
        .set_border(FormatBorder::Thin)
        .set_border_color(Color::RGB(0xd7e1dc))
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap();
    let aisle = Format::new()
        .set_background_color(Color::RGB(0xeef1f4))
        .set_align(FormatAlign::Center);
    let side = matches!(data.podium, "left" | "right");
    let column_offset = u16::from(data.podium == "left");
    let last = data.columns - 1 + u16::from(side);
    let offset = u32::from(data.podium_front);
    let write_heading = |grid: &mut rust_xlsxwriter::Worksheet, row, label: &str| {
        if last == 0 {
            grid.write_string_with_format(row, 0, label, &title).map(|_| ())
        } else {
            grid.merge_range(row, 0, row, last, label, &title).map(|_| ())
        }
        .map_err(|e| e.to_string())
    };
    write_heading(grid, 0, &data.class_name)?;
    if side {
        let column = if column_offset == 1 { 0 } else { last };
        if data.rows == 1 { grid.write_string_with_format(1, column, "讲台", &title) }
        else { grid.merge_range(1, column, u32::from(data.rows), column, "讲台", &title) }
            .map_err(|e| e.to_string())?;
    } else {
        write_heading(grid, if data.podium_front { 1 } else { u32::from(data.rows) + 2 }, "讲台")?;
    }
    grid.set_landscape().set_paper_size(9);
    grid.set_column_range_width(0, last, 18)
        .map_err(|e| e.to_string())?;
    for row in 1..=data.rows {
        grid.set_row_height(u32::from(row) + offset, 68)
            .map_err(|e| e.to_string())?;
    }
    for seat in &data.cells {
        let label = match seat.kind {
            CellKind::Seat => {
                let mut label = format!(
                    "{}排 {}座\n{}",
                    seat.row,
                    seat.column,
                    seat.name.as_deref().unwrap_or("空座")
                );
                if let Some(number) = &seat.number {
                    label.push_str(&format!("\n{number}"));
                }
                if seat.locked == Some(true) {
                    label.push_str(" · 锁定");
                }
                label
            }
            CellKind::Aisle => "过道".to_string(),
            CellKind::Unavailable => "不可用".to_string(),
        };
        grid.write_string_with_format(
            u32::from(seat.row) + offset,
            seat.column - 1 + column_offset,
            label,
            if seat.kind == CellKind::Seat {
                &cell
            } else {
                &aisle
            },
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(ratings) = &data.ratings {
        let sheet = workbook.add_worksheet();
        sheet.set_name("规则评分").map_err(|e| e.to_string())?;
        sheet.set_column_width(0, 35).map_err(|e| e.to_string())?;
        for (column, label) in ["规则", "得分", "满分"].iter().enumerate() {
            sheet
                .write_string(0, column as u16, *label)
                .map_err(|e| e.to_string())?;
        }
        for (index, rating) in ratings.iter().enumerate() {
            let row = index as u32 + 1;
            sheet
                .write_string(row, 0, &rating.label)
                .map_err(|e| e.to_string())?;
            sheet
                .write_number(row, 1, rating.score)
                .map_err(|e| e.to_string())?;
            sheet
                .write_number(row, 2, rating.maximum)
                .map_err(|e| e.to_string())?;
        }
    }
    workbook.save_to_buffer().map_err(|e| e.to_string())
}

pub fn save_file<R: tauri::Runtime>(
    window: WebviewWindow<R>,
    bytes: Vec<u8>,
    extension: &'static str,
) -> Result<bool, String> {
    let chosen = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("另存为座位表")
        .set_file_name(format!("座位表.{extension}"))
        .add_filter(extension.to_uppercase(), &[extension])
        .save_file();
    let Some(path) = chosen else {
        return Ok(false);
    };
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(extension))
    {
        return Err(format!("请选择 .{extension} 文件名"));
    }
    let parent = path.parent().ok_or("保存路径无效")?;
    let write_error = |e: std::io::Error| format!("保存座位表失败：{e}");
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(write_error)?;
    temporary.write_all(&bytes).map_err(write_error)?;
    temporary.as_file().sync_all().map_err(write_error)?;
    temporary.persist(&path).map_err(|e| write_error(e.error))?;
    Ok(true)
}
