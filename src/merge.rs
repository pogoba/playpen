use clap::Args;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::collections::HashMap;
use std::error::Error;
use std::ffi::CString;
use std::fs;
use std::io::IsTerminal;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Args)]
pub struct MergeArgs {
    /// Overlay upperdir (source of changes).
    #[arg(long)]
    pub upper: PathBuf,

    /// Overlay lowerdir (target of merge).
    #[arg(long)]
    pub lower: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChangeKind {
    Added,
    Modified,
    Deleted,
    OpaqueDir,
}

impl ChangeKind {
    fn letter(self) -> &'static str {
        match self {
            ChangeKind::Added => "A",
            ChangeKind::Modified => "M",
            ChangeKind::Deleted => "D",
            ChangeKind::OpaqueDir => "O",
        }
    }
    fn color(self) -> Color {
        match self {
            ChangeKind::Added => Color::Green,
            ChangeKind::Modified => Color::Yellow,
            ChangeKind::Deleted => Color::Red,
            ChangeKind::OpaqueDir => Color::Magenta,
        }
    }
}

struct Entry {
    rel_path: PathBuf,
    kind: ChangeKind,
    is_dir: bool,
    selected: bool,
}

struct TreeNode {
    name: String,
    full_path: PathBuf,
    depth: usize,
    is_dir: bool,
    expanded: bool,
    entry_idx: Option<usize>,
    children: Vec<usize>,
}

struct Tree {
    nodes: Vec<TreeNode>,
    roots: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionState {
    None,
    Partial,
    All,
}

fn build_tree(entries: &[Entry]) -> Tree {
    let mut tree = Tree {
        nodes: Vec::new(),
        roots: Vec::new(),
    };
    let mut by_path: HashMap<PathBuf, usize> = HashMap::new();

    for (idx, entry) in entries.iter().enumerate() {
        let mut accum = PathBuf::new();
        let comps: Vec<_> = entry.rel_path.components().collect();
        if comps.is_empty() {
            continue;
        }
        let last = comps.len() - 1;

        for (i, comp) in comps.iter().enumerate() {
            accum.push(comp.as_os_str());
            let is_last = i == last;
            let entry_for_node = if is_last { Some(idx) } else { None };
            let want_dir = !is_last || entry.is_dir;
            let depth = i;

            let parent_idx = if i == 0 {
                None
            } else {
                let mut parent_path = accum.clone();
                parent_path.pop();
                Some(*by_path.get(&parent_path).expect("parent must exist"))
            };

            if let Some(&existing_idx) = by_path.get(&accum) {
                if entry_for_node.is_some() {
                    tree.nodes[existing_idx].entry_idx = entry_for_node;
                }
                continue;
            }

            let name = comp.as_os_str().to_string_lossy().into_owned();
            let node_idx = tree.nodes.len();
            tree.nodes.push(TreeNode {
                name,
                full_path: accum.clone(),
                depth,
                is_dir: want_dir,
                expanded: true,
                entry_idx: entry_for_node,
                children: Vec::new(),
            });
            by_path.insert(accum.clone(), node_idx);

            match parent_idx {
                Some(p) => tree.nodes[p].children.push(node_idx),
                None => tree.roots.push(node_idx),
            }
        }
    }

    // Sort each node's children: dirs first, then alphabetical.
    fn sort_children(tree: &mut Tree, node_idx: usize) {
        let mut child_ids: Vec<usize> = tree.nodes[node_idx].children.clone();
        child_ids.sort_by(|&a, &b| {
            let na = &tree.nodes[a];
            let nb = &tree.nodes[b];
            nb.is_dir
                .cmp(&na.is_dir)
                .then_with(|| na.name.cmp(&nb.name))
        });
        tree.nodes[node_idx].children = child_ids.clone();
        for c in child_ids {
            sort_children(tree, c);
        }
    }
    let roots = tree.roots.clone();
    tree.roots.sort_by(|&a, &b| {
        let na = &tree.nodes[a];
        let nb = &tree.nodes[b];
        nb.is_dir
            .cmp(&na.is_dir)
            .then_with(|| na.name.cmp(&nb.name))
    });
    for r in roots {
        sort_children(&mut tree, r);
    }
    tree
}

fn rebuild_visible(tree: &Tree, visible: &mut Vec<usize>) {
    visible.clear();
    fn walk(tree: &Tree, idx: usize, out: &mut Vec<usize>) {
        out.push(idx);
        let node = &tree.nodes[idx];
        if node.is_dir && node.expanded {
            for &c in &node.children {
                walk(tree, c, out);
            }
        }
    }
    for &root in &tree.roots {
        walk(tree, root, visible);
    }
}

fn collect_descendant_entries(tree: &Tree, node_idx: usize, out: &mut Vec<usize>) {
    let node = &tree.nodes[node_idx];
    if let Some(i) = node.entry_idx {
        out.push(i);
    }
    for &c in &node.children {
        collect_descendant_entries(tree, c, out);
    }
}

fn dir_selection_state(tree: &Tree, entries: &[Entry], node_idx: usize) -> SelectionState {
    let mut idxs = Vec::new();
    collect_descendant_entries(tree, node_idx, &mut idxs);
    if idxs.is_empty() {
        return SelectionState::None;
    }
    let sel = idxs.iter().filter(|&&i| entries[i].selected).count();
    if sel == 0 {
        SelectionState::None
    } else if sel == idxs.len() {
        SelectionState::All
    } else {
        SelectionState::Partial
    }
}

fn toggle_node_selection(tree: &Tree, entries: &mut [Entry], node_idx: usize) {
    let node = &tree.nodes[node_idx];
    if !node.is_dir {
        if let Some(i) = node.entry_idx {
            entries[i].selected = !entries[i].selected;
        }
        return;
    }
    let mut idxs = Vec::new();
    collect_descendant_entries(tree, node_idx, &mut idxs);
    if idxs.is_empty() {
        return;
    }
    let all = idxs.iter().all(|&i| entries[i].selected);
    let target = !all;
    for i in idxs {
        entries[i].selected = target;
    }
}

pub fn run(args: MergeArgs) -> Result<(), Box<dyn Error>> {
    if !args.upper.is_dir() {
        return Err(format!("upper layer is not a directory: {}", args.upper.display()).into());
    }
    if !args.lower.is_dir() {
        return Err(format!("lower layer is not a directory: {}", args.lower.display()).into());
    }

    let mut entries = scan(&args.upper, &args.lower)?;
    if entries.is_empty() {
        println!("No changes in upper layer.");
        return Ok(());
    }
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    if !std::io::stdout().is_terminal() {
        return Err("merge requires a TTY for the interactive picker".into());
    }

    let selected = match tui_loop(entries, &args.upper, &args.lower)? {
        Some(s) => s,
        None => {
            println!("Aborted.");
            return Ok(());
        }
    };

    if selected.is_empty() {
        println!("Nothing selected.");
        return Ok(());
    }

    for entry in &selected {
        apply(entry, &args.upper, &args.lower)
            .map_err(|e| format!("apply {}: {}", entry.rel_path.display(), e))?;
    }

    let mut clear_order: Vec<&Entry> = selected.iter().collect();
    clear_order.sort_by(|a, b| b.rel_path.cmp(&a.rel_path));
    for entry in clear_order {
        if let Err(err) = clear_from_upper(entry, &args.upper) {
            if err.raw_os_error() != Some(libc::ENOTEMPTY) {
                eprintln!(
                    "warning: failed to remove {} from upper: {}",
                    entry.rel_path.display(),
                    err
                );
            }
        }
    }

    let n = selected.len();
    println!(
        "Applied {} entr{} to {}",
        n,
        if n == 1 { "y" } else { "ies" },
        args.lower.display()
    );
    Ok(())
}

fn scan(upper: &Path, lower: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    scan_recursive(upper, lower, Path::new(""), &mut out)?;
    Ok(out)
}

fn scan_recursive(
    upper: &Path,
    lower: &Path,
    rel: &Path,
    out: &mut Vec<Entry>,
) -> std::io::Result<()> {
    let upper_dir = upper.join(rel);
    for dirent in fs::read_dir(&upper_dir)? {
        let dirent = dirent?;
        let name = dirent.file_name();
        let child_rel = rel.join(&name);
        let upper_path = upper.join(&child_rel);
        let lower_path = lower.join(&child_rel);
        let meta = fs::symlink_metadata(&upper_path)?;
        let ftype = meta.file_type();

        if is_whiteout(&meta) {
            out.push(Entry {
                rel_path: child_rel,
                kind: ChangeKind::Deleted,
                is_dir: false,
                selected: false,
            });
            continue;
        }

        if ftype.is_dir() {
            let opaque = read_opaque_xattr(&upper_path).unwrap_or(false);
            if opaque {
                out.push(Entry {
                    rel_path: child_rel.clone(),
                    kind: ChangeKind::OpaqueDir,
                    is_dir: true,
                    selected: false,
                });
            } else if !path_exists(&lower_path) {
                out.push(Entry {
                    rel_path: child_rel.clone(),
                    kind: ChangeKind::Added,
                    is_dir: true,
                    selected: false,
                });
            }
            // Recurse to pick up children regardless of whether the dir
            // itself emitted an entry.
            scan_recursive(upper, lower, &child_rel, out)?;
            continue;
        }

        let kind = if path_exists(&lower_path) {
            ChangeKind::Modified
        } else {
            ChangeKind::Added
        };
        out.push(Entry {
            rel_path: child_rel,
            kind,
            is_dir: false,
            selected: false,
        });
    }
    Ok(())
}

fn path_exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

fn is_whiteout(meta: &fs::Metadata) -> bool {
    meta.file_type().is_char_device() && meta.rdev() == 0
}

fn read_opaque_xattr(path: &Path) -> std::io::Result<bool> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let name = CString::new("trusted.overlay.opaque").unwrap();
    let mut buf = [0u8; 4];
    let len = unsafe {
        libc::lgetxattr(
            c_path.as_ptr(),
            name.as_ptr(),
            buf.as_mut_ptr() as *mut _,
            buf.len(),
        )
    };
    if len < 0 {
        let err = std::io::Error::last_os_error();
        let errno = err.raw_os_error().unwrap_or(0);
        if errno == libc::ENODATA || errno == libc::ENOTSUP || errno == libc::EOPNOTSUPP {
            return Ok(false);
        }
        return Err(err);
    }
    Ok(len > 0 && buf[0] == b'y')
}

fn diff_text(entry: &Entry, upper: &Path, lower: &Path) -> String {
    match entry.kind {
        ChangeKind::Deleted => format!(
            "-- whiteout: {} will be removed from the lower layer\n",
            entry.rel_path.display()
        ),
        ChangeKind::OpaqueDir => format!(
            "-- opaque dir: lower contents at {} will be replaced wholesale\n",
            entry.rel_path.display()
        ),
        ChangeKind::Added | ChangeKind::Modified => {
            if entry.is_dir {
                return format!("-- new directory: {}\n", entry.rel_path.display());
            }
            let upper_path = upper.join(&entry.rel_path);
            let lower_path = lower.join(&entry.rel_path);
            let lower_arg: PathBuf = if matches!(entry.kind, ChangeKind::Added) {
                PathBuf::from("/dev/null")
            } else {
                lower_path
            };
            match Command::new("diff")
                .arg("-u")
                .arg("--")
                .arg(&lower_arg)
                .arg(&upper_path)
                .output()
            {
                Ok(out) => {
                    let s = String::from_utf8_lossy(&out.stdout).into_owned();
                    if s.is_empty() {
                        "-- files compare equal --\n".into()
                    } else {
                        sanitize_for_tui(&s)
                    }
                }
                Err(err) => format!("(could not run diff: {})\n", err),
            }
        }
    }
}

fn sanitize_for_tui(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut col: usize = 0;
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                out.push('\n');
                col = 0;
            }
            '\t' => {
                // Expand to next multiple of 8 so columns stay aligned.
                let pad = 8 - (col % 8);
                for _ in 0..pad {
                    out.push(' ');
                }
                col += pad;
            }
            '\x1b' => match chars.next() {
                Some('[') => {
                    while let Some(&c2) = chars.peek() {
                        chars.next();
                        if (0x40..=0x7e).contains(&(c2 as u32)) {
                            break;
                        }
                    }
                }
                Some(']') => loop {
                    match chars.next() {
                        None | Some('\x07') => break,
                        Some('\x1b') => {
                            if matches!(chars.peek(), Some('\\')) {
                                chars.next();
                            }
                            break;
                        }
                        _ => {}
                    }
                },
                Some(_) | None => {}
            },
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                out.push('·');
                col += 1;
            }
            _ => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

fn apply(entry: &Entry, upper: &Path, lower: &Path) -> std::io::Result<()> {
    let upper_path = upper.join(&entry.rel_path);
    let lower_path = lower.join(&entry.rel_path);

    match entry.kind {
        ChangeKind::Deleted => match fs::symlink_metadata(&lower_path) {
            Ok(meta) => {
                if meta.file_type().is_dir() {
                    fs::remove_dir_all(&lower_path)?;
                } else {
                    fs::remove_file(&lower_path)?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        },
        ChangeKind::OpaqueDir => {
            if path_exists(&lower_path) {
                fs::remove_dir_all(&lower_path)?;
            }
            create_parent(&lower_path)?;
            fs::create_dir(&lower_path)?;
            copy_meta(&upper_path, &lower_path).ok();
        }
        ChangeKind::Added | ChangeKind::Modified => {
            let meta = fs::symlink_metadata(&upper_path)?;
            let ftype = meta.file_type();
            create_parent(&lower_path)?;
            if ftype.is_dir() {
                if !path_exists(&lower_path) {
                    fs::create_dir(&lower_path)?;
                }
                copy_meta(&upper_path, &lower_path).ok();
            } else if ftype.is_symlink() {
                let target = fs::read_link(&upper_path)?;
                if path_exists(&lower_path) {
                    fs::remove_file(&lower_path)?;
                }
                symlink(&target, &lower_path)?;
            } else {
                fs::copy(&upper_path, &lower_path)?;
                copy_meta(&upper_path, &lower_path).ok();
            }
        }
    }
    Ok(())
}

fn clear_from_upper(entry: &Entry, upper: &Path) -> std::io::Result<()> {
    let upper_path = upper.join(&entry.rel_path);
    let meta = match fs::symlink_metadata(&upper_path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if meta.file_type().is_dir() {
        fs::remove_dir(&upper_path)
    } else {
        fs::remove_file(&upper_path)
    }
}

fn create_parent(p: &Path) -> std::io::Result<()> {
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn copy_meta(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    fs::set_permissions(dst, meta.permissions()).ok();
    let times = [
        libc::timespec {
            tv_sec: meta.atime() as libc::time_t,
            tv_nsec: meta.atime_nsec() as libc::c_long,
        },
        libc::timespec {
            tv_sec: meta.mtime() as libc::time_t,
            tv_nsec: meta.mtime_nsec() as libc::c_long,
        },
    ];
    let c_path = CString::new(dst.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    unsafe {
        libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0);
    }
    Ok(())
}

struct AppState {
    entries: Vec<Entry>,
    tree: Tree,
    visible: Vec<usize>,
    cursor: usize,
    list_state: ListState,
    diff_offset: u16,
    diff_cache: HashMap<usize, String>,
    upper: PathBuf,
    lower: PathBuf,
}

fn current_node(state: &AppState) -> usize {
    state.visible[state.cursor.min(state.visible.len() - 1)]
}

fn refresh_visible_keep_cursor(state: &mut AppState) {
    let prev_node = if state.visible.is_empty() {
        None
    } else {
        Some(state.visible[state.cursor.min(state.visible.len() - 1)])
    };
    rebuild_visible(&state.tree, &mut state.visible);
    if let Some(node) = prev_node {
        if let Some(pos) = state.visible.iter().position(|&n| n == node) {
            state.cursor = pos;
            return;
        }
    }
    state.cursor = state.cursor.min(state.visible.len().saturating_sub(1));
}

fn tui_loop(
    entries: Vec<Entry>,
    upper: &Path,
    lower: &Path,
) -> Result<Option<Vec<Entry>>, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let tree = build_tree(&entries);
    let mut visible = Vec::new();
    rebuild_visible(&tree, &mut visible);

    let mut list_state = ListState::default();
    list_state.select(Some(0));
    let mut state = AppState {
        entries,
        tree,
        visible,
        cursor: 0,
        list_state,
        diff_offset: 0,
        diff_cache: HashMap::new(),
        upper: upper.to_path_buf(),
        lower: lower.to_path_buf(),
    };

    let mut decision: Option<bool> = None;

    let loop_result: Result<(), Box<dyn Error>> = loop {
        let node_idx = current_node(&state);
        if !state.diff_cache.contains_key(&node_idx) {
            let text = node_diff_text(&state, node_idx);
            state.diff_cache.insert(node_idx, text);
        }
        state.list_state.select(Some(state.cursor));
        if let Err(e) = terminal.draw(|f| draw(f, &mut state)) {
            break Err(Box::new(e));
        }

        let evt = match event::read() {
            Ok(e) => e,
            Err(e) => break Err(Box::new(e)),
        };
        if let Event::Key(key) = evt {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                    decision = Some(false);
                    break Ok(());
                }
                (KeyCode::Char('A'), _) => {
                    decision = Some(true);
                    break Ok(());
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    if state.cursor > 0 {
                        state.cursor -= 1;
                        state.diff_offset = 0;
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if state.cursor + 1 < state.visible.len() {
                        state.cursor += 1;
                        state.diff_offset = 0;
                    }
                }
                (KeyCode::Char(' '), _) => {
                    let n = current_node(&state);
                    toggle_node_selection(&state.tree, &mut state.entries, n);
                }
                (KeyCode::Enter, _) | (KeyCode::Tab, _) => {
                    let n = current_node(&state);
                    if state.tree.nodes[n].is_dir {
                        state.tree.nodes[n].expanded = !state.tree.nodes[n].expanded;
                        refresh_visible_keep_cursor(&mut state);
                    }
                }
                (KeyCode::Right, _) | (KeyCode::Char('l'), _) => {
                    let n = current_node(&state);
                    if state.tree.nodes[n].is_dir && !state.tree.nodes[n].expanded {
                        state.tree.nodes[n].expanded = true;
                        refresh_visible_keep_cursor(&mut state);
                    } else if state.cursor + 1 < state.visible.len() {
                        state.cursor += 1;
                        state.diff_offset = 0;
                    }
                }
                (KeyCode::Left, _) | (KeyCode::Char('h'), _) => {
                    let n = current_node(&state);
                    if state.tree.nodes[n].is_dir && state.tree.nodes[n].expanded {
                        state.tree.nodes[n].expanded = false;
                        refresh_visible_keep_cursor(&mut state);
                    } else {
                        // jump to parent dir if any
                        let depth = state.tree.nodes[n].depth;
                        if depth > 0 {
                            for i in (0..state.cursor).rev() {
                                let cand = state.visible[i];
                                if state.tree.nodes[cand].depth < depth {
                                    state.cursor = i;
                                    state.diff_offset = 0;
                                    break;
                                }
                            }
                        }
                    }
                }
                (KeyCode::Char('a'), m) if !m.contains(KeyModifiers::CONTROL) => {
                    let all = state.entries.iter().all(|e| e.selected);
                    for e in state.entries.iter_mut() {
                        e.selected = !all;
                    }
                }
                (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) => {
                    state.diff_offset = state.diff_offset.saturating_add(10);
                }
                (KeyCode::Char('u'), m) if m.contains(KeyModifiers::CONTROL) => {
                    state.diff_offset = state.diff_offset.saturating_sub(10);
                }
                _ => {}
            }
        }
    };

    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .ok();
    terminal.show_cursor().ok();

    loop_result?;

    match decision {
        Some(true) => Ok(Some(
            state.entries.into_iter().filter(|e| e.selected).collect(),
        )),
        _ => Ok(None),
    }
}

fn node_diff_text(state: &AppState, node_idx: usize) -> String {
    let node = &state.tree.nodes[node_idx];
    if let Some(idx) = node.entry_idx {
        return diff_text(&state.entries[idx], &state.upper, &state.lower);
    }
    // Implicit dir: no own entry; summarise descendants.
    let mut idxs = Vec::new();
    collect_descendant_entries(&state.tree, node_idx, &mut idxs);
    if idxs.is_empty() {
        return format!("-- empty subtree at {}\n", node.full_path.display());
    }
    let mut counts = [0usize; 4];
    for &i in &idxs {
        let k = match state.entries[i].kind {
            ChangeKind::Added => 0,
            ChangeKind::Modified => 1,
            ChangeKind::Deleted => 2,
            ChangeKind::OpaqueDir => 3,
        };
        counts[k] += 1;
    }
    let mut s = format!("-- directory: {}\n", node.full_path.display());
    s.push_str(&format!(
        "   {} added · {} modified · {} deleted · {} opaque\n\n",
        counts[0], counts[1], counts[2], counts[3]
    ));
    for &i in &idxs {
        let e = &state.entries[i];
        s.push_str(&format!("   {} {}\n", e.kind.letter(), e.rel_path.display()));
    }
    s
}

fn draw(f: &mut ratatui::Frame, state: &mut AppState) {
    let area = f.area();
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(main[0]);

    let items: Vec<ListItem> = state
        .visible
        .iter()
        .map(|&n| {
            let node = &state.tree.nodes[n];
            let indent = "  ".repeat(node.depth);
            let mut spans: Vec<Span> = Vec::new();
            spans.push(Span::raw(indent));

            if node.is_dir {
                let marker = if node.expanded { "▾" } else { "▸" };
                spans.push(Span::styled(
                    format!("{} ", marker),
                    Style::default().fg(Color::Blue),
                ));
                let sel = dir_selection_state(&state.tree, &state.entries, n);
                let check = match sel {
                    SelectionState::All => "[x]",
                    SelectionState::Partial => "[~]",
                    SelectionState::None => "[ ]",
                };
                spans.push(Span::raw(format!("{} ", check)));
                if let Some(idx) = node.entry_idx {
                    let e = &state.entries[idx];
                    spans.push(Span::styled(
                        format!("{} ", e.kind.letter()),
                        Style::default()
                            .fg(e.kind.color())
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                spans.push(Span::styled(
                    format!("{}/", node.name),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw("  "));
                let sel_flag = node
                    .entry_idx
                    .map(|i| state.entries[i].selected)
                    .unwrap_or(false);
                let check = if sel_flag { "[x]" } else { "[ ]" };
                spans.push(Span::raw(format!("{} ", check)));
                if let Some(idx) = node.entry_idx {
                    let e = &state.entries[idx];
                    spans.push(Span::styled(
                        format!("{} ", e.kind.letter()),
                        Style::default()
                            .fg(e.kind.color())
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                spans.push(Span::raw(node.name.clone()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let selected_count = state.entries.iter().filter(|e| e.selected).count();
    let title = format!(
        " Changes ({} selected / {} total) ",
        selected_count,
        state.entries.len()
    );
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, panes[0], &mut state.list_state);

    let cur_node = state.visible[state.cursor.min(state.visible.len() - 1)];
    let diff = state
        .diff_cache
        .get(&cur_node)
        .cloned()
        .unwrap_or_else(|| "(loading)".to_string());
    let diff_lines: Vec<Line> = diff
        .lines()
        .map(|l| {
            let style = if l.starts_with("+++") || l.starts_with("---") {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if l.starts_with("@@") {
                Style::default().fg(Color::Cyan)
            } else if l.starts_with('+') {
                Style::default().fg(Color::Green)
            } else if l.starts_with('-') {
                Style::default().fg(Color::Red)
            } else if l.starts_with("--") {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            Line::from(Span::styled(l.to_string(), style))
        })
        .collect();
    let diff_widget = Paragraph::new(diff_lines)
        .block(Block::default().borders(Borders::ALL).title(" Diff "))
        .scroll((state.diff_offset, 0));
    f.render_widget(diff_widget, panes[1]);

    let help = Paragraph::new(Line::from(vec![Span::styled(
        " j/k nav · h/l fold · space toggle (recursive) · a all · A apply · ^d/^u diff scroll · q quit",
        Style::default().fg(Color::DarkGray),
    )]));
    f.render_widget(help, main[1]);
}

