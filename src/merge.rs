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
use std::io::{IsTerminal, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Args)]
pub struct MergeArgs {
    /// Named overlay (resolves --upper to its upperdir; --lower defaults
    /// to `/`). Mutually exclusive with `--upper`.
    #[arg(conflicts_with = "upper")]
    pub name: Option<String>,

    /// Overlay upperdir (source of changes). Required when NAME is omitted.
    #[arg(long)]
    pub upper: Option<PathBuf>,

    /// Overlay lowerdir (target of merge). Defaults to `/` when NAME is given;
    /// otherwise required.
    #[arg(long)]
    pub lower: Option<PathBuf>,

    /// Invert the merge direction: write the host's (lower) version of each
    /// selected entry through the overlay mountpoint, so the live sandbox
    /// view picks up the host content. Requires NAME.
    #[arg(long)]
    pub from_host: bool,
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

/// One rendered row. With path collapsing, a row can represent a chain of
/// single-child directories (e.g. `foo/bar/baz/`); `head` is the topmost
/// node in the chain and `tail` is the deepest. Selection acts on the
/// whole subtree (head and tail give the same descendants by the collapse
/// rule); expansion acts on the tail (its children appear below this row).
struct VisibleRow {
    head: usize,
    tail: usize,
    chain: Vec<usize>,
    /// Per-ancestor "is last visible sibling" flag, root-first. Drives the
    /// `│  ` vs `   ` segments of the tree connector.
    ancestor_last_flags: Vec<bool>,
    is_last: bool,
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
            let hidden = name.starts_with('.');
            let node_idx = tree.nodes.len();
            tree.nodes.push(TreeNode {
                name,
                full_path: accum.clone(),
                depth,
                is_dir: want_dir,
                expanded: !hidden,
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

/// Mark gitignored directories as collapsed by default. For each dir
/// node, walk up the on-disk parent chain (rooted at `lower`) collecting
/// `.gitignore` files and ask each whether the dir is ignored. Hidden
/// dirs (already collapsed by `build_tree`) are skipped to avoid
/// redundant I/O.
fn apply_ignore_rules(tree: &mut Tree, lower: &Path) {
    use ignore::gitignore::Gitignore;
    use std::collections::HashMap;

    let lower_canon = lower.canonicalize().unwrap_or_else(|_| lower.to_path_buf());
    // Cache parsed `.gitignore` per containing dir; `None` = no file there.
    let mut cache: HashMap<PathBuf, Option<Gitignore>> = HashMap::new();

    fn lookup<'a>(
        cache: &'a mut HashMap<PathBuf, Option<Gitignore>>,
        dir: &Path,
    ) -> Option<&'a Gitignore> {
        if !cache.contains_key(dir) {
            let gi_path = dir.join(".gitignore");
            let val = if gi_path.is_file() {
                let (gi, _err) = Gitignore::new(&gi_path);
                Some(gi)
            } else {
                None
            };
            cache.insert(dir.to_path_buf(), val);
        }
        cache.get(dir).and_then(|o| o.as_ref())
    }

    let n = tree.nodes.len();
    for i in 0..n {
        if !tree.nodes[i].is_dir || !tree.nodes[i].expanded {
            continue;
        }
        let abs = lower_canon.join(&tree.nodes[i].full_path);
        let mut ignored = false;
        let mut cur = abs.parent();
        while let Some(p) = cur {
            if let Some(gi) = lookup(&mut cache, p) {
                let m = gi.matched_path_or_any_parents(&abs, true);
                if m.is_ignore() {
                    ignored = true;
                    break;
                }
                if m.is_whitelist() {
                    break;
                }
            }
            cur = p.parent();
        }
        if ignored {
            tree.nodes[i].expanded = false;
        }
    }
}

fn rebuild_visible(tree: &Tree, visible: &mut Vec<VisibleRow>) {
    visible.clear();
    fn walk(
        tree: &Tree,
        node_idx: usize,
        ancestor_flags: &[bool],
        is_last: bool,
        out: &mut Vec<VisibleRow>,
    ) {
        // Extend the chain through single-child implicit dirs.
        // Stops if cur is not a dir, is collapsed, has its own entry
        // (would lose the entry letter), or has != 1 child.
        let mut chain = vec![node_idx];
        let mut cur = node_idx;
        loop {
            let n = &tree.nodes[cur];
            if !n.is_dir {
                break;
            }
            if !n.expanded {
                break;
            }
            if n.entry_idx.is_some() {
                break;
            }
            if n.children.len() != 1 {
                break;
            }
            let only = n.children[0];
            chain.push(only);
            cur = only;
        }
        out.push(VisibleRow {
            head: node_idx,
            tail: cur,
            chain,
            ancestor_last_flags: ancestor_flags.to_vec(),
            is_last,
        });
        let tail_node = &tree.nodes[cur];
        if tail_node.is_dir && tail_node.expanded {
            let mut new_flags = ancestor_flags.to_vec();
            new_flags.push(is_last);
            let n_children = tail_node.children.len();
            for (i, &c) in tail_node.children.iter().enumerate() {
                walk(tree, c, &new_flags, i + 1 == n_children, out);
            }
        }
    }
    let n_roots = tree.roots.len();
    for (i, &r) in tree.roots.iter().enumerate() {
        walk(tree, r, &[], i + 1 == n_roots, visible);
    }
}

fn chain_display_name(tree: &Tree, chain: &[usize]) -> String {
    let mut s = String::new();
    for (i, &idx) in chain.iter().enumerate() {
        if i > 0 {
            s.push('/');
        }
        s.push_str(&tree.nodes[idx].name);
    }
    if tree.nodes[*chain.last().unwrap()].is_dir {
        s.push('/');
    }
    s
}

fn connector_string(ancestor_last_flags: &[bool], is_last: bool) -> String {
    let mut s = String::new();
    for &flag in ancestor_last_flags {
        s.push_str(if flag { "   " } else { "│  " });
    }
    s.push_str(if is_last { "└─ " } else { "├─ " });
    s
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
    let resolved = resolve_layers(&args)?;
    let upper = &resolved.upper;
    let lower = &resolved.lower;

    if !upper.is_dir() {
        return Err(format!("upper layer is not a directory: {}", upper.display()).into());
    }
    if !lower.is_dir() {
        return Err(format!("lower layer is not a directory: {}", lower.display()).into());
    }

    let mut entries = scan(upper, lower)?;
    if entries.is_empty() {
        println!("No changes in upper layer.");
        return Ok(());
    }
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    if !std::io::stdout().is_terminal() {
        return Err("merge requires a TTY for the interactive picker".into());
    }

    let selected = match tui_loop(entries, upper, lower)? {
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

    let dest_label = match &resolved.mount {
        Some(m) => m.clone(),
        None => lower.clone(),
    };

    for entry in &selected {
        match &resolved.mount {
            Some(mount) => apply_from_host(entry, lower, mount).map_err(|e| {
                format!("apply --from-host {}: {}", entry.rel_path.display(), e)
            })?,
            None => apply(entry, upper, lower)
                .map_err(|e| format!("apply {}: {}", entry.rel_path.display(), e))?,
        }
    }

    let n = selected.len();
    println!(
        "Applied {} entr{} to {}",
        n,
        if n == 1 { "y" } else { "ies" },
        dest_label.display()
    );
    Ok(())
}

struct ResolvedLayers {
    upper: PathBuf,
    lower: PathBuf,
    /// `Some(mountpoint)` when running in `--from-host` mode; the apply
    /// loop writes through this path instead of into `lower`.
    mount: Option<PathBuf>,
}

fn resolve_layers(args: &MergeArgs) -> Result<ResolvedLayers, Box<dyn Error>> {
    if args.from_host {
        let name = args
            .name
            .as_deref()
            .ok_or("--from-host requires a NAME (the named overlay to write into)")?;
        let upper = crate::overlay::upper_path(name)?;
        let mount = crate::overlay::mount_path(name)?;
        let lower = args.lower.clone().unwrap_or_else(|| PathBuf::from("/"));
        return Ok(ResolvedLayers {
            upper,
            lower,
            mount: Some(mount),
        });
    }
    if let Some(name) = &args.name {
        let upper = crate::overlay::upper_path(name)?;
        let lower = args.lower.clone().unwrap_or_else(|| PathBuf::from("/"));
        return Ok(ResolvedLayers {
            upper,
            lower,
            mount: None,
        });
    }
    let upper = args
        .upper
        .clone()
        .ok_or("merge needs either NAME (positional) or --upper/--lower")?;
    let lower = args
        .lower
        .clone()
        .ok_or("merge needs --lower when --upper is given without a NAME")?;
    Ok(ResolvedLayers {
        upper,
        lower,
        mount: None,
    })
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
            // Skip whiteouts that target a path that doesn't exist in lower —
            // applying would be a no-op.
            if !path_exists(&lower_path) {
                continue;
            }
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
            // Hide entries whose upper content already matches lower —
            // there is nothing to merge.
            if paths_content_equal(&upper_path, &lower_path).unwrap_or(false) {
                continue;
            }
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

fn paths_content_equal(a: &Path, b: &Path) -> std::io::Result<bool> {
    let ma = fs::symlink_metadata(a)?;
    let mb = fs::symlink_metadata(b)?;
    let ta = ma.file_type();
    let tb = mb.file_type();
    if ta.is_symlink() != tb.is_symlink() {
        return Ok(false);
    }
    if ta.is_symlink() {
        return Ok(fs::read_link(a)? == fs::read_link(b)?);
    }
    if !ta.is_file() || !tb.is_file() {
        return Ok(false);
    }
    if ma.len() != mb.len() {
        return Ok(false);
    }
    let mut fa = fs::File::open(a)?;
    let mut fb = fs::File::open(b)?;
    let mut ba = [0u8; 8192];
    let mut bb = [0u8; 8192];
    loop {
        let na = fa.read(&mut ba)?;
        let nb = fb.read(&mut bb)?;
        if na != nb {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
        if ba[..na] != bb[..nb] {
            return Ok(false);
        }
    }
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

/// Inverse of `apply`: write the host's view of each entry through the
/// overlay mountpoint so the live sandbox sees host content. Goes through
/// the overlay's normal write path, which keeps overlayfs caches coherent
/// for any attached `enter` sessions (unlike scribbling on upperdir
/// directly, which would be UB per the overlayfs contract).
fn apply_from_host(entry: &Entry, lower: &Path, mount: &Path) -> std::io::Result<()> {
    let lower_path = lower.join(&entry.rel_path);
    let mount_dst = mount.join(&entry.rel_path);

    match entry.kind {
        ChangeKind::Added => {
            // Upper has it, lower doesn't. Drop it from the sandbox view by
            // removing through the overlay mount.
            match fs::symlink_metadata(&mount_dst) {
                Ok(meta) => {
                    if meta.file_type().is_dir() {
                        fs::remove_dir_all(&mount_dst)?;
                    } else {
                        fs::remove_file(&mount_dst)?;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        ChangeKind::Modified | ChangeKind::Deleted => {
            // Lower is the host-truth version; copy it through the overlay
            // mount so the sandbox picks up host content. For Deleted the
            // overlay currently has a whiteout — writing creates a real
            // upper file that masks it.
            let meta = fs::symlink_metadata(&lower_path)?;
            let ftype = meta.file_type();
            create_parent(&mount_dst)?;
            if ftype.is_symlink() {
                let target = fs::read_link(&lower_path)?;
                if path_exists(&mount_dst) {
                    fs::remove_file(&mount_dst)?;
                }
                symlink(&target, &mount_dst)?;
            } else if ftype.is_dir() {
                if !path_exists(&mount_dst) {
                    fs::create_dir_all(&mount_dst)?;
                }
                copy_meta(&lower_path, &mount_dst).ok();
            } else {
                fs::copy(&lower_path, &mount_dst)?;
                copy_meta(&lower_path, &mount_dst).ok();
            }
        }
        ChangeKind::OpaqueDir => {
            // Removing the trusted.overlay.opaque xattr requires touching
            // upperdir behind the overlay's back; rm-then-recopy through
            // the mount would whiteout the lower contents instead of
            // exposing them. Bail rather than do the wrong thing silently.
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "OpaqueDir entries cannot be reverted via --from-host (would whiteout host contents); revert manually",
            ));
        }
    }
    Ok(())
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
    visible: Vec<VisibleRow>,
    cursor: usize,
    list_state: ListState,
    diff_offset: u16,
    diff_cache: HashMap<usize, String>,
    upper: PathBuf,
    lower: PathBuf,
}

fn current_row(state: &AppState) -> &VisibleRow {
    &state.visible[state.cursor.min(state.visible.len() - 1)]
}

fn refresh_visible_keep_cursor(state: &mut AppState) {
    let prev_tail = if state.visible.is_empty() {
        None
    } else {
        Some(state.visible[state.cursor.min(state.visible.len() - 1)].tail)
    };
    rebuild_visible(&state.tree, &mut state.visible);
    if let Some(tail) = prev_tail {
        if let Some(pos) = state.visible.iter().position(|r| r.chain.contains(&tail)) {
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

    let mut tree = build_tree(&entries);
    apply_ignore_rules(&mut tree, lower);
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
        let tail_idx = current_row(&state).tail;
        if !state.diff_cache.contains_key(&tail_idx) {
            let text = node_diff_text(&state, tail_idx);
            state.diff_cache.insert(tail_idx, text);
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
                    let head = current_row(&state).head;
                    toggle_node_selection(&state.tree, &mut state.entries, head);
                }
                (KeyCode::Enter, _) | (KeyCode::Tab, _) => {
                    let tail = current_row(&state).tail;
                    if state.tree.nodes[tail].is_dir {
                        state.tree.nodes[tail].expanded = !state.tree.nodes[tail].expanded;
                        refresh_visible_keep_cursor(&mut state);
                    }
                }
                (KeyCode::Right, _) | (KeyCode::Char('l'), _) => {
                    let tail = current_row(&state).tail;
                    if state.tree.nodes[tail].is_dir && !state.tree.nodes[tail].expanded {
                        state.tree.nodes[tail].expanded = true;
                        refresh_visible_keep_cursor(&mut state);
                    } else if state.cursor + 1 < state.visible.len() {
                        state.cursor += 1;
                        state.diff_offset = 0;
                    }
                }
                (KeyCode::Left, _) | (KeyCode::Char('h'), _) => {
                    let tail = current_row(&state).tail;
                    let head = current_row(&state).head;
                    if state.tree.nodes[tail].is_dir && state.tree.nodes[tail].expanded {
                        state.tree.nodes[tail].expanded = false;
                        refresh_visible_keep_cursor(&mut state);
                    } else {
                        // jump to parent dir if any (compare by head depth,
                        // which is the row's effective indentation level)
                        let depth = state.tree.nodes[head].depth;
                        if depth > 0 {
                            for i in (0..state.cursor).rev() {
                                let cand_head = state.visible[i].head;
                                if state.tree.nodes[cand_head].depth < depth {
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
        .map(|row| {
            let tail = &state.tree.nodes[row.tail];
            let mut spans: Vec<Span> = Vec::new();

            // Letter gutter (fixed 2 chars: letter + space, or two spaces).
            if let Some(idx) = tail.entry_idx {
                let e = &state.entries[idx];
                spans.push(Span::styled(
                    format!("{} ", e.kind.letter()),
                    Style::default()
                        .fg(e.kind.color())
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw("  "));
            }

            // Tree connector based on this row's visible position. Dim
            // colour so the connectors recede behind the names.
            spans.push(Span::styled(
                connector_string(&row.ancestor_last_flags, row.is_last),
                Style::default().fg(Color::DarkGray),
            ));

            // Expansion marker for the tail dir.
            if tail.is_dir {
                let marker = if tail.expanded { "▾ " } else { "▸ " };
                spans.push(Span::styled(
                    marker.to_string(),
                    Style::default().fg(Color::Blue),
                ));
            }

            // Selection marker (before the name). Only rendered when
            // selected/partial; unselected rows get no leading marker.
            let sel = if tail.is_dir {
                dir_selection_state(&state.tree, &state.entries, row.head)
            } else {
                let s = tail
                    .entry_idx
                    .map(|i| state.entries[i].selected)
                    .unwrap_or(false);
                if s {
                    SelectionState::All
                } else {
                    SelectionState::None
                }
            };
            match sel {
                SelectionState::All => spans.push(Span::raw("[x] ")),
                SelectionState::Partial => spans.push(Span::raw("[~] ")),
                SelectionState::None => {}
            }

            // Full chain name. Trailing `/` already appended by the helper
            // when the tail is a directory.
            let name = chain_display_name(&state.tree, &row.chain);
            if tail.is_dir {
                spans.push(Span::styled(
                    name,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(name));
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

    let cur_tail = state.visible[state.cursor.min(state.visible.len() - 1)].tail;
    let diff = state
        .diff_cache
        .get(&cur_tail)
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

