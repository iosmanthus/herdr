//! BSP tree layout for tiling panes within a workspace.

use ratatui::layout::{Direction, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PaneId(u32);

/// Global atomic counter for unique PaneId generation across all workspaces.
static NEXT_PANE_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl PaneId {
    /// Allocate a globally unique PaneId.
    pub fn alloc() -> Self {
        Self(NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    pub fn raw(self) -> u32 {
        self.0
    }

    /// Reconstruct from a saved u32 (persistence only).
    pub fn from_raw(id: u32) -> Self {
        Self(id)
    }
}

/// Snapshot of a pane's position and focus state after layout.
#[derive(Clone)]
pub struct PaneInfo {
    pub id: PaneId,
    /// Outer rect (including borders if present).
    pub rect: Rect,
    /// Inner rect (content area, excluding borders). Used for selection.
    pub inner_rect: Rect,
    /// Visible scrollbar lane, when scrollback is present. `inner_rect` may still
    /// exclude a stable hidden gutter when this is `None`.
    pub scrollbar_rect: Option<Rect>,
    pub is_focused: bool,
}

/// Info about a split boundary, used for mouse drag resize.
#[derive(Clone)]
pub struct SplitBorder {
    /// Position of the divider line (x for horizontal split, y for vertical).
    pub pos: u16,
    /// Direction of the split that created this border.
    pub direction: Direction,
    /// Total area of the split node.
    pub area: Rect,
    /// Path from root to this split node (false=first, true=second).
    pub path: Vec<bool>,
}

/// Cardinal direction for pane navigation.
#[derive(Debug, Clone, Copy)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

/// A node in the BSP tree. Public for serialization.
pub enum Node {
    Pane(PaneId),
    Split {
        direction: Direction,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// BSP tiling layout. Tracks a tree of splits and a focused pane.
pub struct TileLayout {
    root: Node,
    focus: PaneId,
}

impl TileLayout {
    /// Create a new layout with a single pane (globally unique ID).
    /// Returns (layout, root_pane_id) so the caller can create the pane.
    pub fn new() -> (Self, PaneId) {
        let root_id = PaneId::alloc();
        (
            Self {
                root: Node::Pane(root_id),
                focus: root_id,
            },
            root_id,
        )
    }

    pub fn focused(&self) -> PaneId {
        self.focus
    }

    pub fn pane_count(&self) -> usize {
        count_panes(&self.root)
    }

    /// Compute rects for all panes given the available area.
    pub fn panes(&self, area: Rect) -> Vec<PaneInfo> {
        self.panes_with_gap(area, 0)
    }

    /// Compute pane rects, leaving `gap` blank cells between adjacent panes.
    /// `gap == 0` produces touching rects (identical to [`panes`]).
    pub fn panes_with_gap(&self, area: Rect, gap: u16) -> Vec<PaneInfo> {
        let mut result = Vec::new();
        collect_panes(&self.root, area, self.focus, gap, &mut result);
        result
    }

    /// Collect all split boundaries for mouse drag resize.
    pub fn splits(&self, area: Rect) -> Vec<SplitBorder> {
        self.splits_with_gap(area, 0)
    }

    /// Collect split boundaries accounting for the inter-pane `gap`.
    pub fn splits_with_gap(&self, area: Rect, gap: u16) -> Vec<SplitBorder> {
        let mut result = Vec::new();
        collect_splits(&self.root, area, vec![], gap, &mut result);
        result
    }

    /// Split the focused pane. Returns the new pane's id.
    pub fn split_focused(&mut self, direction: Direction) -> PaneId {
        let new_id = PaneId::alloc();
        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        self.root = split_at(old, self.focus, direction, new_id);
        self.focus = new_id;
        new_id
    }

    /// Close the focused pane. Returns false if it's the last pane.
    pub fn close_focused(&mut self) -> bool {
        if self.pane_count() <= 1 {
            return false;
        }
        let target = self.focus;
        let ids = self.pane_ids();
        let pos = ids.iter().position(|id| *id == target).unwrap();
        let new_focus = if pos + 1 < ids.len() {
            ids[pos + 1]
        } else {
            ids[pos - 1]
        };
        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        if let Some(new_root) = remove_pane(old, target) {
            self.root = new_root;
            self.focus = new_focus;
            true
        } else {
            false
        }
    }

    pub fn focus_pane(&mut self, id: PaneId) {
        if self.pane_ids().contains(&id) {
            self.focus = id;
        }
    }

    /// Set the ratio of a split node at the given path.
    pub fn set_ratio_at(&mut self, path: &[bool], ratio: f32) {
        set_ratio_at(&mut self.root, path, ratio.clamp(0.1, 0.9));
    }

    /// Adjust the nearest split in the given direction for the focused pane.
    /// `delta` is positive to grow, negative to shrink.
    pub fn resize_focused(&mut self, nav: NavDirection, delta: f32, area: Rect, gap: u16) {
        let panes = self.panes_with_gap(area, gap);
        let Some(focused) = panes.iter().find(|p| p.is_focused) else {
            return;
        };
        let focused_rect = focused.rect;
        let splits = self.splits_with_gap(area, gap);

        // Find the split whose border is adjacent to the focused pane in the given direction
        let target_dir = match nav {
            NavDirection::Left | NavDirection::Right => Direction::Horizontal,
            NavDirection::Up | NavDirection::Down => Direction::Vertical,
        };
        let grows = matches!(nav, NavDirection::Right | NavDirection::Down);

        // Find the closest matching split border
        let best = splits
            .iter()
            .filter(|s| s.direction == target_dir)
            .filter(|s| match target_dir {
                Direction::Horizontal => {
                    // Border must be near the focused pane's left or right edge
                    let near_right = (s.pos as i32 - (focused_rect.x + focused_rect.width) as i32)
                        .unsigned_abs()
                        <= 1;
                    let near_left = (s.pos as i32 - focused_rect.x as i32).unsigned_abs() <= 1;
                    near_right || near_left
                }
                Direction::Vertical => {
                    let near_bottom = (s.pos as i32
                        - (focused_rect.y + focused_rect.height) as i32)
                        .unsigned_abs()
                        <= 1;
                    let near_top = (s.pos as i32 - focused_rect.y as i32).unsigned_abs() <= 1;
                    near_bottom || near_top
                }
            })
            .min_by_key(|s| {
                // Prefer the border in the direction we're resizing toward
                match (target_dir, grows) {
                    (Direction::Horizontal, true) => {
                        ((focused_rect.x + focused_rect.width) as i32 - s.pos as i32).unsigned_abs()
                    }
                    (Direction::Horizontal, false) => {
                        (focused_rect.x as i32 - s.pos as i32).unsigned_abs()
                    }
                    (Direction::Vertical, true) => ((focused_rect.y + focused_rect.height) as i32
                        - s.pos as i32)
                        .unsigned_abs(),
                    (Direction::Vertical, false) => {
                        (focused_rect.y as i32 - s.pos as i32).unsigned_abs()
                    }
                }
            });

        if let Some(split) = best {
            let path = split.path.clone();
            let current_ratio = get_ratio_at(&self.root, &path).unwrap_or(0.5);
            let adj = if grows { delta } else { -delta };
            self.set_ratio_at(&path, current_ratio + adj);
        }
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        collect_ids(&self.root, &mut ids);
        ids
    }

    /// Access the tree root for serialization.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Reconstruct a layout from a saved tree.
    /// Reconstruct a layout from a saved tree.
    pub fn from_saved(root: Node, focus: PaneId) -> Self {
        Self { root, focus }
    }
}

// --- Directional pane navigation ---

/// Find the nearest pane in the given direction from `focused`.
pub fn find_in_direction(
    focused: &PaneInfo,
    direction: NavDirection,
    panes: &[PaneInfo],
) -> Option<PaneId> {
    let fr = focused.rect;

    panes
        .iter()
        .filter(|p| p.id != focused.id)
        .filter(|p| {
            let r = p.rect;
            match direction {
                NavDirection::Left => {
                    r.x + r.width <= fr.x && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Right => {
                    r.x >= fr.x + fr.width && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up => {
                    r.y + r.height <= fr.y && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
                NavDirection::Down => {
                    r.y >= fr.y + fr.height && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
            }
        })
        .min_by_key(|p| {
            let r = p.rect;
            match direction {
                NavDirection::Left => fr.x.saturating_sub(r.x + r.width),
                NavDirection::Right => r.x.saturating_sub(fr.x + fr.width),
                NavDirection::Up => fr.y.saturating_sub(r.y + r.height),
                NavDirection::Down => r.y.saturating_sub(fr.y + fr.height),
            }
        })
        .map(|p| p.id)
}

fn ranges_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start + b_len && a_start + a_len > b_start
}

// --- Tree operations ---

fn count_panes(node: &Node) -> usize {
    match node {
        Node::Pane(_) => 1,
        Node::Split { first, second, .. } => count_panes(first) + count_panes(second),
    }
}

fn collect_panes(node: &Node, area: Rect, focus: PaneId, gap: u16, result: &mut Vec<PaneInfo>) {
    match node {
        Node::Pane(id) => {
            result.push(PaneInfo {
                id: *id,
                rect: area,
                // inner_rect is set during render when we know if borders are shown
                inner_rect: area,
                scrollbar_rect: None,
                is_focused: *id == focus,
            });
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (a, b) = split_rect(area, *direction, *ratio, gap);
            collect_panes(first, a, focus, gap, result);
            collect_panes(second, b, focus, gap, result);
        }
    }
}

fn collect_splits(node: &Node, area: Rect, path: Vec<bool>, gap: u16, result: &mut Vec<SplitBorder>) {
    if let Node::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    {
        let (a, b) = split_rect(area, *direction, *ratio, gap);
        let pos = match direction {
            Direction::Horizontal => a.x + a.width,
            Direction::Vertical => a.y + a.height,
        };
        result.push(SplitBorder {
            pos,
            direction: *direction,
            area,
            path: path.clone(),
        });
        let mut lp = path.clone();
        lp.push(false);
        collect_splits(first, a, lp, gap, result);
        let mut rp = path;
        rp.push(true);
        collect_splits(second, b, rp, gap, result);
    }
}

fn collect_ids(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Pane(id) => ids.push(*id),
        Node::Split { first, second, .. } => {
            collect_ids(first, ids);
            collect_ids(second, ids);
        }
    }
}

fn split_at(node: Node, target: PaneId, direction: Direction, new_id: PaneId) -> Node {
    match node {
        Node::Pane(id) if id == target => Node::Split {
            direction,
            ratio: 0.5,
            first: Box::new(Node::Pane(id)),
            second: Box::new(Node::Pane(new_id)),
        },
        Node::Pane(_) => node,
        Node::Split {
            direction: d,
            ratio,
            first,
            second,
        } => Node::Split {
            direction: d,
            ratio,
            first: Box::new(split_at(*first, target, direction, new_id)),
            second: Box::new(split_at(*second, target, direction, new_id)),
        },
    }
}

fn remove_pane(node: Node, target: PaneId) -> Option<Node> {
    match node {
        Node::Pane(id) if id == target => None,
        Node::Pane(_) => Some(node),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => match (remove_pane(*first, target), remove_pane(*second, target)) {
            (None, Some(s)) => Some(s),
            (Some(f), None) => Some(f),
            (Some(f), Some(s)) => Some(Node::Split {
                direction,
                ratio,
                first: Box::new(f),
                second: Box::new(s),
            }),
            (None, None) => None,
        },
    }
}

fn set_ratio_at(node: &mut Node, path: &[bool], new_ratio: f32) {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            *ratio = new_ratio;
        } else if path[0] {
            set_ratio_at(second, &path[1..], new_ratio);
        } else {
            set_ratio_at(first, &path[1..], new_ratio);
        }
    }
}

fn get_ratio_at(node: &Node, path: &[bool]) -> Option<f32> {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            Some(*ratio)
        } else if path[0] {
            get_ratio_at(second, &path[1..])
        } else {
            get_ratio_at(first, &path[1..])
        }
    } else {
        None
    }
}

/// Split `area` into two child rects. `gap` blank cells are left between them;
/// `gap == 0` yields touching rects (the historical behaviour).
fn split_rect(area: Rect, direction: Direction, ratio: f32, gap: u16) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let first_w = ((area.width as f32) * ratio).round() as u16;
            let second_x = area.x.saturating_add(first_w).saturating_add(gap);
            let second_w = area.width.saturating_sub(first_w).saturating_sub(gap);
            (
                Rect::new(area.x, area.y, first_w, area.height),
                Rect::new(second_x, area.y, second_w, area.height),
            )
        }
        Direction::Vertical => {
            let first_h = ((area.height as f32) * ratio).round() as u16;
            let second_y = area.y.saturating_add(first_h).saturating_add(gap);
            let second_h = area.height.saturating_sub(first_h).saturating_sub(gap);
            (
                Rect::new(area.x, area.y, area.width, first_h),
                Rect::new(area.x, second_y, area.width, second_h),
            )
        }
    }
}

#[cfg(test)]
mod gap_tests {
    use super::*;

    #[test]
    fn split_rect_gap_zero_is_touching() {
        let area = Rect::new(0, 0, 100, 30);
        let (a, b) = split_rect(area, Direction::Horizontal, 0.5, 0);
        assert_eq!(a, Rect::new(0, 0, 50, 30));
        assert_eq!(b, Rect::new(50, 0, 50, 30));
        assert_eq!(a.x + a.width, b.x, "panes touch when gap == 0");
    }

    #[test]
    fn split_rect_horizontal_gap_separates_panes() {
        let area = Rect::new(0, 0, 100, 30);
        let (a, b) = split_rect(area, Direction::Horizontal, 0.5, 2);
        assert_eq!(a, Rect::new(0, 0, 50, 30));
        assert_eq!(b, Rect::new(52, 0, 48, 30));
        assert_eq!(b.x - (a.x + a.width), 2, "two blank cells between panes");
    }

    #[test]
    fn split_rect_vertical_gap_separates_panes() {
        let area = Rect::new(0, 0, 80, 24);
        let (a, b) = split_rect(area, Direction::Vertical, 0.5, 1);
        assert_eq!(a, Rect::new(0, 0, 80, 12));
        assert_eq!(b, Rect::new(0, 13, 80, 11));
        assert_eq!(b.y - (a.y + a.height), 1, "one blank row between panes");
    }
}
