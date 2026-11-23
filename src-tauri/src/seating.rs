use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const ENGINE_VERSION: u32 = 1;
const SEARCH_BUDGET: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Student {
    pub id: i64,
    pub number: String,
    pub name: String,
    pub gender: Option<bool>,
    pub height_tenths: Option<u32>,
    pub poor_vision: bool,
    pub discipline_attention: bool,
    pub score_rank: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_level: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
    Seat,
    Aisle,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub id: i64,
    pub row: u16,
    pub column: u16,
    pub kind: CellKind,
    pub desk: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Vision,
    Height,
    Gender,
    Score,
    Discipline,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftRule {
    pub name: String,
    pub kind: RuleKind,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontRule {
    pub name: String,
    pub student: i64,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    NotDeskmate,
    NotAdjacent,
    PreferDeskmate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    pub first: i64,
    pub second: i64,
    pub kind: RelationKind,
    #[serde(default = "default_relation_weight")]
    pub weight: u32,
}

fn default_relation_weight() -> u32 { 100 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodiumSide { Left, Right }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_version: u32,
    pub engine_version: u32,
    pub class_id: i64,
    pub class_name: String,
    pub classroom_id: i64,
    pub classroom_name: String,
    pub exam_id: Option<i64>,
    pub exam_name: Option<String>,
    pub rows: u16,
    pub columns: u16,
    pub podium_front: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub podium_side: Option<PodiumSide>,
    #[serde(default)]
    pub use_score_levels: bool,
    pub students: Vec<Student>,
    pub positions: Vec<Position>,
    pub front_rules: Vec<FrontRule>,
    pub soft_rules: Vec<SoftRule>,
    pub relations: Vec<Relation>,
    pub warnings: Vec<String>,
}

impl Snapshot {
    pub fn podium(&self) -> &'static str {
        match self.podium_side {
            Some(PodiumSide::Left) => "left",
            Some(PodiumSide::Right) => "right",
            None if self.podium_front => "front",
            None => "back",
        }
    }

    pub fn depth(&self, position: &Position) -> u16 {
        match self.podium_side {
            Some(PodiumSide::Left) => position.column,
            Some(PodiumSide::Right) => self.columns - position.column + 1,
            None if self.podium_front => position.row,
            None => self.rows - position.row + 1,
        }
    }

    fn maximum_depth(&self) -> u16 {
        if self.podium_side.is_some() { self.columns } else { self.rows }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub student_id: i64,
    pub position_id: i64,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rating {
    pub label: String,
    pub score: u32,
    pub maximum: u32,
}

fn adjacent(a: &Position, b: &Position) -> bool {
    u32::from(a.row.abs_diff(b.row)) + u32::from(a.column.abs_diff(b.column)) == 1
}

fn same_desk(a: &Position, b: &Position) -> bool {
    !a.desk.is_empty() && a.desk == b.desk
}

struct Engine<'a> {
    input: &'a Snapshot,
    seats: Vec<&'a Position>,
    distance: Vec<u16>,
    peer_seats: Vec<Vec<usize>>,
    students: HashMap<i64, usize>,
    seat_ids: HashMap<i64, usize>,
    front: Vec<u16>,
    hard: Vec<Vec<(usize, RelationKind)>>,
    preferred: Vec<(usize, usize, u32)>,
    height_target: Vec<Option<u32>>,
    rank_span: u32,
}

impl<'a> Engine<'a> {
    fn new(input: &'a Snapshot) -> Result<Self, String> {
        if input.snapshot_version != 1 || input.engine_version != ENGINE_VERSION {
            return Err("此方案的快照或排座引擎版本暂不支持编辑".into());
        }
        if input.soft_rules.iter().any(|rule| rule.weight > 1000) || input.relations.iter().any(|rule| rule.weight > 1000) {
            return Err("软规则权重超出范围".into());
        }
        let seats: Vec<_> = input
            .positions
            .iter()
            .filter(|p| p.kind == CellKind::Seat)
            .collect();
        if input.students.is_empty() {
            return Err("班级没有在用学生".into());
        }
        let students: HashMap<_, _> = input
            .students
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id, i))
            .collect();
        let seat_ids: HashMap<_, _> = seats.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
        let coordinates: HashMap<_, _> = seats
            .iter()
            .enumerate()
            .map(|(i, p)| ((p.row, p.column), i))
            .collect();
        if students.len() != input.students.len()
            || coordinates.len() != seats.len()
            || seat_ids.len() != seats.len()
            || input.rows == 0
            || input.columns == 0
            || input.rows > 50
            || input.columns > 50
            || seats.iter().any(|p| {
                !(1..=input.rows).contains(&p.row) || !(1..=input.columns).contains(&p.column)
            })
        {
            return Err("方案包含无效或重复的学生或座位".into());
        }
        let distance = seats
            .iter()
            .map(|p| input.depth(p))
            .collect();
        let mut desks = BTreeMap::<&str, Vec<usize>>::new();
        for (index, seat) in seats.iter().enumerate() {
            if !seat.desk.is_empty() {
                desks.entry(&seat.desk).or_default().push(index);
            }
        }
        let peer_seats = seats
            .iter()
            .enumerate()
            .map(|(index, seat)| {
                if let Some(group) = desks.get(seat.desk.as_str()) {
                    if group.len() > 1 {
                        return group
                            .iter()
                            .copied()
                            .filter(|&peer| peer != index)
                            .collect();
                    }
                }
                [
                    (seat.row - 1, seat.column),
                    (seat.row + 1, seat.column),
                    (seat.row, seat.column - 1),
                    (seat.row, seat.column + 1),
                ]
                .iter()
                .filter_map(|coordinate| coordinates.get(coordinate).copied())
                .collect()
            })
            .collect();
        let mut front = vec![input.maximum_depth(); students.len()];
        for rule in &input.front_rules {
            let index = *students
                .get(&rule.student)
                .ok_or_else(|| format!("前排规则「{}」引用了不在当前名单的学生", rule.name))?;
            if rule.rows == 0 {
                return Err(format!("前排规则「{}」的排数无效", rule.name));
            }
            front[index] = front[index].min(rule.rows);
        }
        let mut hard = vec![Vec::new(); students.len()];
        let mut preferred = Vec::new();
        for relation in &input.relations {
            let (Some(&a), Some(&b)) = (
                students.get(&relation.first),
                students.get(&relation.second),
            ) else {
                continue;
            };
            if a == b {
                return Err("学生关系引用了同一名学生".into());
            }
            if relation.kind == RelationKind::PreferDeskmate {
                preferred.push((a, b, relation.weight));
            } else {
                hard[a].push((b, relation.kind));
                hard[b].push((a, relation.kind));
            }
        }
        let heights: Vec<_> = input
            .students
            .iter()
            .filter_map(|s| s.height_tenths)
            .collect();
        let height_target = input
            .students
            .iter()
            .map(|s| {
                let height = s.height_tenths?;
                let min = *heights.iter().min()?;
                let max = *heights.iter().max()?;
                (max > min).then(|| ((u64::from(height - min) * 100) / u64::from(max - min)) as u32)
            })
            .collect();
        let rank_span = input
            .students
            .iter()
            .filter_map(|s| s.score_rank)
            .max()
            .unwrap_or(1)
            .saturating_sub(1)
            .max(1);
        Ok(Self {
            input,
            seats,
            distance,
            peer_seats,
            students,
            seat_ids,
            front,
            hard,
            preferred,
            height_target,
            rank_span,
        })
    }

    fn indices(
        &self,
        assignments: &[Assignment],
        locked_only: bool,
    ) -> Result<Vec<Option<usize>>, String> {
        let mut result = vec![None; self.students.len()];
        let mut occupied = vec![false; self.seats.len()];
        for item in assignments.iter().filter(|a| !locked_only || a.locked) {
            let student = *self
                .students
                .get(&item.student_id)
                .ok_or("分配引用了未知学生")?;
            let seat = *self
                .seat_ids
                .get(&item.position_id)
                .ok_or("分配或锁座指向不可用位置")?;
            if result[student].is_some() || occupied[seat] {
                return Err("学生或座位出现重复分配或锁定".into());
            }
            result[student] = Some(seat);
            occupied[seat] = true;
        }
        Ok(result)
    }

    fn violation(
        &self,
        student: usize,
        seat: usize,
        positions: &[Option<usize>],
    ) -> Option<String> {
        let name = &self.input.students[student].name;
        if self.distance[seat] > self.front[student] {
            return Some(format!(
                "「{name}」须坐在靠讲台的前 {} 排",
                self.front[student]
            ));
        }
        for &(other, kind) in &self.hard[student] {
            let Some(other_seat) = positions[other] else {
                continue;
            };
            let (a, b) = (self.seats[seat], self.seats[other_seat]);
            let description = match kind {
                RelationKind::NotAdjacent if adjacent(a, b) => "不能相邻",
                RelationKind::NotDeskmate if same_desk(a, b) => "不能同桌",
                _ => continue,
            };
            return Some(format!(
                "「{name}」与「{}」{description}",
                self.input.students[other].name
            ));
        }
        None
    }

    fn permits(&self, student: usize, seat: usize, positions: &[Option<usize>]) -> bool {
        self.distance[seat] <= self.front[student]
            && self.hard[student].iter().all(|&(other, kind)| {
                positions[other].is_none_or(|position| match kind {
                    RelationKind::NotAdjacent => !adjacent(self.seats[seat], self.seats[position]),
                    RelationKind::NotDeskmate => !same_desk(self.seats[seat], self.seats[position]),
                    RelationKind::PreferDeskmate => true,
                })
            })
    }

    fn ratings(&self, placements: &[Option<usize>]) -> Vec<Rating> {
        let mut occupants = vec![None; self.seats.len()];
        for (student, &seat) in placements.iter().enumerate() {
            if let Some(seat) = seat { occupants[seat] = Some(student); }
        }
        let span = u32::from(self.input.maximum_depth().saturating_sub(1)).max(1);
        let mut ratings = Vec::new();
        for rule in &self.input.soft_rules {
            let mut earned = 0;
            let mut eligible = 0;
            for (index, student) in self.input.students.iter().enumerate() {
                let Some(seat) = placements[index] else { continue };
                let depth = u32::from(self.distance[seat] - 1) * 100 / span;
                let peers = || {
                    self.peer_seats[seat]
                        .iter()
                        .filter_map(|&p| occupants[p])
                        .map(|p| &self.input.students[p])
                };
                let points = match rule.kind {
                    RuleKind::Vision if student.poor_vision => 100 - depth,
                    RuleKind::Height => {
                        let Some(target) = self.height_target[index] else {
                            continue;
                        };
                        if self.input.maximum_depth() == 1 {
                            100
                        } else {
                            100 - depth.abs_diff(target)
                        }
                    }
                    RuleKind::Gender => {
                        let Some(gender) = student.gender else {
                            continue;
                        };
                        if peers().any(|peer| peer.gender.is_some_and(|other| other != gender)) {
                            100
                        } else {
                            0
                        }
                    }
                    RuleKind::Score => {
                        if self.input.use_score_levels {
                            let Some(level) = student.score_level else { continue };
                            let mut levels = peers().filter_map(|peer| peer.score_level).peekable();
                            if levels.peek().is_none() { continue; }
                            let score = if levels.any(|other| other == level) { 0 } else { 100 };
                            earned += score;
                            eligible += 1;
                            continue;
                        }
                        let Some(rank) = student.score_rank else {
                            continue;
                        };
                        peers()
                            .filter_map(|peer| {
                                peer.score_rank.map(|other| {
                                    (u64::from(rank.abs_diff(other)) * 100
                                        / u64::from(self.rank_span))
                                        as u32
                                })
                            })
                            .max()
                            .unwrap_or(0)
                    }
                    RuleKind::Discipline if student.discipline_attention => {
                        if peers().any(|peer| peer.discipline_attention) {
                            0
                        } else {
                            100
                        }
                    }
                    _ => continue,
                };
                eligible += 1;
                earned += points;
            }
            ratings.push(Rating {
                label: rule.name.clone(),
                score: (earned * rule.weight).checked_div(eligible).unwrap_or(0),
                maximum: if eligible == 0 { 0 } else { rule.weight * 100 },
            });
        }
        let mut preference_points = (0_u64, 0_u64);
        for &(a, b, weight) in &self.preferred {
            if let (Some(a), Some(b)) = (placements[a], placements[b]) {
                preference_points.1 += u64::from(weight);
                if same_desk(self.seats[a], self.seats[b]) { preference_points.0 += u64::from(weight); }
            }
        }
        if preference_points.1 > 0 {
            ratings.push(Rating {
                label: "偏好同桌".into(),
                score: (100 * preference_points.0 / preference_points.1) as u32,
                maximum: 100,
            });
        }
        ratings
    }
}

pub fn validate(input: &Snapshot, assignments: &[Assignment]) -> Result<Vec<Rating>, String> {
    validate_placements(input, assignments, true)
}

pub fn validate_placements(input: &Snapshot, assignments: &[Assignment], complete: bool) -> Result<Vec<Rating>, String> {
    let engine = Engine::new(input)?;
    let placements = engine.indices(assignments, false)?;
    for (student, &seat) in placements.iter().enumerate() {
        let Some(seat) = seat else {
            if complete { return Err(format!("「{}」尚未分配座位", input.students[student].name)); }
            continue;
        };
        if let Some(error) = engine.violation(student, seat, &placements) {
            return Err(error);
        }
    }
    Ok(engine.ratings(&placements))
}

fn seat_empty_priority(engine: &Engine<'_>, seat: usize) -> (u16, u16, u16) {
    let pos = engine.seats[seat];
    (engine.distance[seat], pos.row, pos.column)
}

fn solve_with_allowed_seats(
    engine: &Engine<'_>,
    locked_placements: &[Option<usize>],
    locked: &[bool],
    allowed: &[bool],
    seed: u32,
) -> Result<(Vec<Option<usize>>, Vec<Rating>), String> {
    let mut placements = locked_placements.to_vec();
    let mut occupied = vec![false; engine.seats.len()];
    for &seat in placements.iter().flatten() {
        occupied[seat] = true;
    }
    for rows in 1..engine.input.maximum_depth() {
        let required = engine
            .front
            .iter()
            .enumerate()
            .filter(|&(student, &limit)| !locked[student] && limit <= rows)
            .count();
        let capacity = engine
            .distance
            .iter()
            .enumerate()
            .filter(|&(seat, &depth)| allowed[seat] && !occupied[seat] && depth <= rows)
            .count();
        if required > capacity {
            return Err(format!(
                "靠讲台前 {rows} 排需要 {required} 个未锁座位，实际只有 {capacity} 个"
            ));
        }
    }
    let mut random = u64::from(seed) + 1;
    let mut next = |range: usize| {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        (random % range as u64) as usize
    };
    let mut order: Vec<_> = (0..engine.students.len()).filter(|&s| !locked[s]).collect();
    let mut choices: Vec<Vec<usize>> = order
        .iter()
        .map(|&s| {
            let mut seats: Vec<_> = (0..engine.seats.len())
                .filter(|&p| allowed[p] && !occupied[p] && engine.permits(s, p, &placements))
                .collect();
            for i in (1..seats.len()).rev() {
                seats.swap(i, next(i + 1));
            }
            seats
        })
        .collect();
    let mut search_order: Vec<_> = (0..order.len()).collect();
    search_order.sort_by_key(|&index| {
        (
            choices[index].len(),
            std::cmp::Reverse(engine.hard[order[index]].len()),
            order[index],
        )
    });
    order = search_order.iter().map(|&index| order[index]).collect();
    choices = search_order
        .iter()
        .map(|&index| std::mem::take(&mut choices[index]))
        .collect();
    let mut cursors = vec![0; order.len()];
    let mut depth = 0;
    let mut attempts = 0;
    while depth < order.len() {
        let student = order[depth];
        if let Some(&seat) = choices[depth].get(cursors[depth]) {
            attempts += 1;
            if attempts > SEARCH_BUDGET {
                return Err("排座搜索预算已用尽，尚未找到合法方案。可更换种子或检查约束".into());
            }
            cursors[depth] += 1;
            if !occupied[seat] && engine.permits(student, seat, &placements) {
                occupied[seat] = true;
                placements[student] = Some(seat);
                depth += 1;
            }
        } else {
            cursors[depth] = 0;
            if depth == 0 {
                let blocked = order
                    .iter()
                    .zip(&choices)
                    .find(|(_, seats)| seats.is_empty())
                    .map(|(&s, _)| {
                        format!(
                            "「{}」没有符合前排、锁座和学生关系的候选座位",
                            engine.input.students[s].name
                        )
                    });
                return Err(blocked
                    .unwrap_or_else(|| "硬规则组合无解，请检查前排限制、锁座与学生关系".into()));
            }
            depth -= 1;
            occupied[placements[order[depth]].take().unwrap()] = false;
        }
    }

    let active_seats: Vec<usize> = (0..engine.seats.len()).filter(|&seat| allowed[seat]).collect();
    let mut ratings = engine.ratings(&placements);
    let mut total: u64 = ratings.iter().map(|r| u64::from(r.score)).sum();
    if !order.is_empty() && !ratings.is_empty() && !active_seats.is_empty() {
        let mut occupants = vec![None; engine.seats.len()];
        for (student, &seat) in placements.iter().enumerate() {
            occupants[seat.unwrap()] = Some(student);
        }
        let peer_work: usize = engine.peer_seats.iter().map(Vec::len).sum();
        let cost = ((engine.input.students.len() + peer_work) * engine.input.soft_rules.len().max(1)
            + engine.preferred.len())
        .max(1);
        let iterations = (2_000_000 / cost).clamp(1, 2000);
        for _ in 0..iterations {
            let student = order[next(order.len())];
            let from = placements[student].unwrap();
            let to = active_seats[next(active_seats.len())];
            let other = occupants[to];
            if from == to || other.is_some_and(|s| locked[s]) {
                continue;
            }
            placements[student] = Some(to);
            if let Some(s) = other {
                placements[s] = Some(from);
            }
            if engine.permits(student, to, &placements)
                && other.is_none_or(|s| engine.permits(s, from, &placements))
            {
                let proposed = engine.ratings(&placements);
                let value = proposed.iter().map(|r| u64::from(r.score)).sum();
                if value > total {
                    total = value;
                    ratings = proposed;
                    occupants[to] = Some(student);
                    occupants[from] = other;
                    continue;
                }
            }
            placements[student] = Some(from);
            if let Some(s) = other {
                placements[s] = Some(to);
            }
        }
    }
    Ok((placements, ratings))
}

pub fn generate(
    input: &Snapshot,
    previous: &[Assignment],
    seed: u32,
) -> Result<(Vec<Assignment>, Vec<Rating>), String> {
    let engine = Engine::new(input)?;
    if engine.seats.len() < input.students.len() {
        return Err(format!("可用座位不足：{} 名学生，只有 {} 个可用座位", input.students.len(), engine.seats.len()));
    }
    let locked_placements = engine.indices(previous, true)?;
    let locked: Vec<_> = locked_placements.iter().map(Option::is_some).collect();
    let mut locked_occupied = vec![false; engine.seats.len()];
    for (student, &seat) in locked_placements.iter().enumerate() {
        if let Some(seat) = seat {
            if let Some(error) = engine.violation(student, seat, &locked_placements) {
                return Err(format!("锁座冲突：{error}"));
            }
            locked_occupied[seat] = true;
        }
    }

    let extra_seats = engine.seats.len().saturating_sub(input.students.len());
    let mut preferred_allowed = vec![true; engine.seats.len()];
    if extra_seats > 0 {
        let mut unlocked_seats: Vec<usize> = (0..engine.seats.len())
            .filter(|&seat| !locked_occupied[seat])
            .collect();
        unlocked_seats.sort_by_key(|&seat| std::cmp::Reverse(seat_empty_priority(&engine, seat)));
        for &seat in unlocked_seats.iter().take(extra_seats) {
            preferred_allowed[seat] = false;
        }
    }

    let (placements, ratings) = match solve_with_allowed_seats(
        &engine,
        &locked_placements,
        &locked,
        &preferred_allowed,
        seed,
    ) {
        Ok(solution) => solution,
        Err(_) if extra_seats > 0 => {
            let all_allowed = vec![true; engine.seats.len()];
            solve_with_allowed_seats(&engine, &locked_placements, &locked, &all_allowed, seed)?
        }
        Err(error) => return Err(error),
    };

    let assignments = input
        .students
        .iter()
        .enumerate()
        .map(|(index, student)| Assignment {
            student_id: student.id,
            position_id: engine.seats[placements[index].unwrap()].id,
            locked: locked[index],
        })
        .collect();
    Ok((assignments, ratings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot(student_count: usize, rows: u16, columns: u16, aisle_cols: &[u16]) -> Snapshot {
        let mut positions = Vec::new();
        let mut id = 1;
        for row in 1..=rows {
            for col in 1..=columns {
                let kind = if aisle_cols.contains(&col) { CellKind::Aisle } else { CellKind::Seat };
                let desk = if kind == CellKind::Seat {
                    let pair = match col {
                        1 | 2 => "A",
                        4 | 5 => "B",
                        7 | 8 => "C",
                        _ => "",
                    };
                    format!("{row}-{pair}")
                } else {
                    String::new()
                };
                positions.push(Position { id, row, column: col, kind, desk });
                id += 1;
            }
        }
        let students = (1..=student_count)
            .map(|i| Student {
                id: i as i64,
                number: format!("{i:03}"),
                name: format!("学生{i}"),
                gender: Some(i % 2 == 0),
                height_tenths: None,
                poor_vision: false,
                discipline_attention: false,
                score_rank: None,
                score_level: None,
            })
            .collect();
        Snapshot {
            snapshot_version: 1,
            engine_version: ENGINE_VERSION,
            class_id: 1,
            class_name: "测试班".into(),
            classroom_id: 1,
            classroom_name: "测试教室".into(),
            exam_id: None,
            exam_name: None,
            rows,
            columns,
            podium_front: true,
            podium_side: None,
            use_score_levels: true,
            students,
            positions,
            front_rules: vec![],
            soft_rules: vec![SoftRule { name: "男女搭配均衡".into(), kind: RuleKind::Gender, weight: 40 }],
            relations: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn extra_empty_seats_default_to_bottom_right_and_respect_locks() {
        let snapshot = sample_snapshot(50, 9, 8, &[3, 6]);
        let (assignments, _) = generate(&snapshot, &[], 42).unwrap();
        let occupied: std::collections::HashSet<i64> = assignments.iter().map(|a| a.position_id).collect();
        let empty_coords: Vec<(u16, u16)> = snapshot
            .positions
            .iter()
            .filter(|p| p.kind == CellKind::Seat && !occupied.contains(&p.id))
            .map(|p| (p.row, p.column))
            .collect();
        assert_eq!(empty_coords, vec![(9, 4), (9, 5), (9, 7), (9, 8)]);

        let corner = snapshot.positions.iter().find(|p| p.row == 9 && p.column == 8).unwrap();
        let locked = vec![Assignment { student_id: 1, position_id: corner.id, locked: true }];
        let (assignments, _) = generate(&snapshot, &locked, 42).unwrap();
        let occupied: std::collections::HashSet<i64> = assignments.iter().map(|a| a.position_id).collect();
        let empty_coords: Vec<(u16, u16)> = snapshot
            .positions
            .iter()
            .filter(|p| p.kind == CellKind::Seat && !occupied.contains(&p.id))
            .map(|p| (p.row, p.column))
            .collect();
        assert_eq!(empty_coords, vec![(9, 2), (9, 4), (9, 5), (9, 7)]);
    }

    #[test]
    fn falls_back_to_all_seats_when_hard_constraints_require_bottom_right() {
        let mut snapshot = sample_snapshot(2, 1, 3, &[]);
        snapshot.positions.iter_mut().for_each(|p| p.desk.clear());
        snapshot.relations.push(Relation { first: 1, second: 2, kind: RelationKind::NotAdjacent, weight: 100 });
        let (assignments, _) = generate(&snapshot, &[], 7).unwrap();
        let occupied_cols: Vec<u16> = assignments
            .iter()
            .map(|a| snapshot.positions.iter().find(|p| p.id == a.position_id).unwrap().column)
            .collect();
        assert!(occupied_cols.contains(&1) && occupied_cols.contains(&3));
    }
}
