//! 提交图的车道：日志里每个提交画在哪一条竖线（车道）上，相邻两行之间的线从哪条车道连到哪条车道。
//!
//! 提交按「孩子在前、父提交在后」的顺序（`git log --date-order`）交给 [`Graph::push`]，状态只有
//! 「每条车道下面等着的是哪个提交」，因此日志分批取回时可以从上次的状态继续计算。一条线从出现
//! 到合进别的线用同一种颜色（[`GraphRow::color`]，按 [`COLORS`] 种轮换），并有不重复的编号。

/// 可用的颜色编号个数（个）。
///
/// 颜色编号在 `0..COLORS` 里轮换。取 8 覆盖常见的并行分支数；
/// 颜色重复时由 [`GraphRow::line`] 区分同色的线。
pub const COLORS: u8 = 8;

/// 提交图中的一段线：从 `from` 车道连到 `to` 车道，画成 `color` 号颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// 起点所在的车道。
    pub from: usize,
    /// 终点所在的车道。
    pub to: usize,
    /// 颜色编号，取值在 `0..COLORS` 之间。
    pub color: u8,
    /// 这段线属于哪条线（[`GraphRow::line`]）。
    pub line: u32,
}

/// 日志里一行的提交图。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphRow {
    /// 提交的圆点画在哪条车道上。
    pub lane: usize,
    /// 圆点的颜色，与所在那条线的颜色相同。
    pub color: u8,
    /// 圆点所在的那条线的编号：一条线从出现到合进别的线（或到根提交）不变，新分出来的线换一个
    /// 新的，不重复。
    pub line: u32,
    /// 上半行：从行的上沿（上一行的车道）连到行的中间（这一行的车道）。
    ///
    /// 直着穿过的线 `from` 等于 `to`；连到圆点的线 `to` 等于 [`GraphRow::lane`]。
    pub up: Vec<Segment>,
    /// 下半行：从行的中间连到行的下沿（下一行的车道）。
    ///
    /// 从圆点出去连到父提交的线 `from` 等于 [`GraphRow::lane`]。
    pub down: Vec<Segment>,
    /// 这一行用到的车道数（条），即画图那一列要留的宽度。
    pub width: usize,
}

/// 一条车道：下面等着的是哪个提交，这条线是什么颜色。
#[derive(Debug, Clone)]
struct Lane {
    /// 车道上等着的下一个提交（`oid`）。
    waiting: String,
    /// 这条线的颜色编号，取值在 `0..COLORS` 之间。
    color: u8,
    /// 这条线的编号，见 [`GraphRow::line`]。
    line: u32,
}

/// 增量计算的提交图。
#[derive(Debug, Clone, Default)]
pub struct Graph {
    /// 各车道的状态；`None` 是空车道。
    lanes: Vec<Option<Lane>>,
    /// 下一条新线用的颜色编号。
    next_color: u8,
    /// 下一条新线的编号。
    next_line: u32,
}

impl Graph {
    /// 创建一个没有任何车道的空图。
    pub fn new() -> Self {
        Graph::default()
    }

    /// 把提交 `oid` 接到图上，返回这一行的画法；`parents` 是提交 `oid` 的父提交，第一个是主线。
    pub fn push(&mut self, oid: &str, parents: &[impl AsRef<str>]) -> GraphRow {
        let mut row = GraphRow::default();

        // 等着 `oid` 的车道：第一条是圆点所在的车道，其余几条（几个分支在这里汇合）合进来以后
        // 空出来。
        let waiting: Vec<usize> = (0..self.lanes.len())
            .filter(|&i| self.lanes[i].as_ref().is_some_and(|l| l.waiting == oid))
            .collect();
        let (lane, color, line) = match waiting.first() {
            Some(&i) => {
                let l = self.lanes[i].as_ref();
                (i, l.map_or(0, |l| l.color), l.map_or(0, |l| l.line))
            }
            // 没有车道等着 `oid`：一个分支的最新提交，开一条新车道。
            None => {
                let (color, line) = (self.new_color(), self.new_line());
                (self.free_lane(), color, line)
            }
        };
        row.lane = lane;
        row.color = color;
        row.line = line;

        // 上半行：等着 `oid` 的线连到圆点，别的线直着穿过。
        for (i, slot) in self.lanes.iter().enumerate() {
            if let Some(l) = slot {
                let to = if l.waiting == oid { lane } else { i };
                row.up.push(Segment {
                    from: i,
                    to,
                    color: l.color,
                    line: l.line,
                });
            }
        }
        for &i in waiting.iter().skip(1) {
            self.lanes[i] = None;
        }
        if lane >= self.lanes.len() {
            self.lanes.resize(lane + 1, None);
        }

        // 圆点下面：第一个父提交继续使用原来的车道和颜色；别的父提交已经有车道等着的连过去，
        // 没有的开新车道、换新颜色。
        let mut parents = parents.iter().map(AsRef::as_ref);
        self.lanes[lane] = parents.next().map(|first| Lane {
            waiting: first.to_owned(),
            color,
            line,
        });
        let mut targets = Vec::new();
        if self.lanes[lane].is_some() {
            targets.push((lane, color, line));
        }
        for parent in parents {
            let existing = self
                .lanes
                .iter()
                .position(|l| l.as_ref().is_some_and(|l| l.waiting == parent));
            match existing {
                Some(i) => {
                    let l = self.lanes[i].as_ref();
                    targets.push((i, l.map_or(color, |l| l.color), l.map_or(line, |l| l.line)));
                }
                None => {
                    let i = self.free_lane();
                    let (new, new_line) = (self.new_color(), self.new_line());
                    if i >= self.lanes.len() {
                        self.lanes.resize(i + 1, None);
                    }
                    self.lanes[i] = Some(Lane {
                        waiting: parent.to_owned(),
                        color: new,
                        line: new_line,
                    });
                    targets.push((i, new, new_line));
                }
            }
        }

        // 下半行：上半行直着穿过的线继续向下穿过（父提交合进来的那条也还在），圆点连到各个
        // 父提交的车道。新开的车道从圆点开始，上面没有线。
        let straight: Vec<Segment> = row
            .up
            .iter()
            .filter(|s| s.from == s.to && s.to != lane)
            .copied()
            .collect();
        row.down.extend(straight);
        for &(to, color, line) in &targets {
            row.down.push(Segment {
                from: lane,
                to,
                color,
                line,
            });
        }

        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
        let used = |s: &Segment| s.from.max(s.to) + 1;
        row.width = row
            .up
            .iter()
            .chain(&row.down)
            .map(used)
            .max()
            .unwrap_or(0)
            .max(lane + 1);
        row
    }

    /// 第一条空着的车道；都占着时是最右边的下一条。
    fn free_lane(&self) -> usize {
        self.lanes
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.lanes.len())
    }

    fn new_line(&mut self) -> u32 {
        let line = self.next_line;
        self.next_line += 1;
        line
    }

    fn new_color(&mut self) -> u8 {
        let color = self.next_color;
        self.next_color = (self.next_color + 1) % COLORS;
        color
    }
}

#[cfg(test)]
mod tests;
