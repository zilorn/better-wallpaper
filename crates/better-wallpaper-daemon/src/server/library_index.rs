//! Shared, lazy library index. Unchanged requests only drain the inotify queue;
//! directory enumeration and project parsing are cached independently.
use std::{collections::HashSet, io};

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

use super::*;

#[derive(Clone, Default)]
struct Directory {
    children: Vec<PathBuf>,
    videos: Vec<LibraryEntry>,
}

#[derive(Clone)]
struct Project {
    entry: Option<LibraryEntry>,
    dependencies: Vec<PathBuf>,
}

pub(super) struct LibraryIndex {
    monitor: Monitor,
    directories: HashMap<PathBuf, Directory>,
    projects: HashMap<PathBuf, Project>,
    snapshot: Option<Arc<LibraryPayload>>,
    #[cfg(test)]
    directory_reads: usize,
    #[cfg(test)]
    project_reads: usize,
}

impl LibraryIndex {
    pub(super) fn new() -> Self {
        Self {
            monitor: Monitor::new(),
            directories: HashMap::new(),
            projects: HashMap::new(),
            snapshot: None,
            #[cfg(test)]
            directory_reads: 0,
            #[cfg(test)]
            project_reads: 0,
        }
    }

    pub(super) fn refresh(&mut self, roots: Vec<PathBuf>, home: &Path) -> Arc<LibraryPayload> {
        let changes = self.monitor.changes();
        if let Some(snapshot) = &self.snapshot
            && snapshot.roots == roots
            && changes.as_ref().is_some_and(|changes| {
                !changes.iter().any(|changed| {
                    self.monitor
                        .targets
                        .iter()
                        .any(|path| related(path, changed))
                })
            })
        {
            return Arc::clone(snapshot);
        }
        match changes {
            Some(changes) => {
                self.directories.retain(|directory, _| {
                    !changes.iter().any(|changed| {
                        changed.parent() == Some(directory.as_path())
                            || directory.starts_with(changed)
                    })
                });
                self.projects
                    .retain(|project, _| !changes.iter().any(|changed| related(project, changed)));
            }
            None => {
                // An overflow or unavailable watch cannot prove cache freshness.
                self.directories.clear();
                self.projects.clear();
            }
        }

        self.monitor.begin_scan();
        let discovery = [
            home.join(".local/share/Steam/steamapps/libraryfolders.vdf"),
            home.join(".steam/steam/steamapps/libraryfolders.vdf"),
            home.join(
                ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/libraryfolders.vdf",
            ),
        ];
        for path in &discovery {
            self.monitor.track(path);
        }
        let candidates = wallpaper_engine_candidates(home);
        for path in &candidates {
            self.monitor.track(path);
        }
        let mut engine_roots: Vec<_> = candidates
            .iter()
            .filter(|path| path.is_dir())
            .filter_map(|path| path.canonicalize().ok())
            .collect();
        engine_roots.sort();
        engine_roots.dedup();
        self.scan(roots, engine_roots)
    }

    pub(super) fn scan(
        &mut self,
        roots: Vec<PathBuf>,
        engine_roots: Vec<PathBuf>,
    ) -> Arc<LibraryPayload> {
        let mut used_directories = HashSet::new();
        let mut used_projects = HashSet::new();
        let mut visited = HashSet::new();
        let mut pending: Vec<_> = roots.iter().map(|path| (path.clone(), 0)).collect();
        let mut entries = Vec::new();
        for root in roots.iter().chain(&engine_roots) {
            self.monitor.track(root);
        }
        while let Some((directory, depth)) = pending.pop() {
            let Ok(canonical) = directory.canonicalize() else {
                continue;
            };
            if !visited.insert(canonical.clone()) || engine_roots.contains(&canonical) {
                continue;
            }
            let listing = self.directory(&canonical, &mut used_directories);
            if depth < MAX_LIBRARY_DEPTH {
                pending.extend(listing.children.into_iter().map(|path| (path, depth + 1)));
            }
            entries.extend(
                listing
                    .videos
                    .into_iter()
                    .take(MAX_LIBRARY_ENTRIES - entries.len()),
            );
            if entries.len() >= MAX_LIBRARY_ENTRIES {
                break;
            }
        }
        if entries.len() < MAX_LIBRARY_ENTRIES {
            'workshops: for root in &engine_roots {
                let listing = self.directory(root, &mut used_directories);
                for project in listing.children {
                    used_projects.insert(project.clone());
                    // Register before reading the descriptor, including projects
                    // whose descriptor/media has not been created yet.
                    self.monitor.track(&project);
                    let cached = if let Some(cached) = self.projects.get(&project) {
                        cached.clone()
                    } else {
                        #[cfg(test)]
                        {
                            self.project_reads += 1;
                        }
                        let mut dependencies = Vec::new();
                        if let Ok(json) = fs::read_to_string(project.join("project.json"))
                            && let Ok(descriptor) =
                                serde_json::from_str::<WallpaperEngineProject>(&json)
                        {
                            for relative in descriptor.file.iter().chain(&descriptor.preview) {
                                if !relative.is_absolute()
                                    && !relative
                                        .components()
                                        .any(|part| part == std::path::Component::ParentDir)
                                {
                                    let dependency = project.join(relative);
                                    self.monitor.track(&dependency);
                                    dependencies.push(dependency);
                                }
                            }
                        }
                        let cached = Project {
                            entry: scan_wallpaper_engine_project(&project),
                            dependencies,
                        };
                        self.projects.insert(project.clone(), cached.clone());
                        cached
                    };
                    for dependency in &cached.dependencies {
                        self.monitor.track(dependency);
                    }
                    if let Some(entry) = cached.entry {
                        entries.push(entry);
                        if entries.len() >= MAX_LIBRARY_ENTRIES {
                            break 'workshops;
                        }
                    }
                }
            }
        }
        self.directories
            .retain(|path, _| used_directories.contains(path));
        self.projects.retain(|path, _| used_projects.contains(path));
        self.monitor.end_scan();
        let truncated = entries.len() >= MAX_LIBRARY_ENTRIES;
        entries.sort_by_key(|entry| entry.name.to_lowercase());
        let by_path = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), index))
            .collect();
        info!(
            entries = entries.len(),
            directories = self.directories.len(),
            projects = self.projects.len(),
            truncated,
            "library index updated"
        );
        let snapshot = Arc::new(LibraryPayload {
            entries,
            roots,
            engine_roots,
            truncated,
            by_path,
        });
        self.snapshot = Some(Arc::clone(&snapshot));
        snapshot
    }

    fn directory(&mut self, path: &Path, used: &mut HashSet<PathBuf>) -> Directory {
        used.insert(path.to_path_buf());
        self.monitor.track(path);
        if let Some(directory) = self.directories.get(path) {
            return directory.clone();
        }
        #[cfg(test)]
        {
            self.directory_reads += 1;
        }
        let mut directory = Directory::default();
        let children = match fs::read_dir(path) {
            Ok(children) => children,
            Err(error) => {
                warn!(path = %path.display(), %error, "skipping unreadable library directory");
                return directory;
            }
        };
        for child in children.flatten() {
            let path = child.path();
            let Ok(kind) = child.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                directory.children.push(path);
            } else if kind.is_file()
                && is_video_path(&path)
                && path.to_str().is_some()
                && directory.videos.len() < MAX_LIBRARY_ENTRIES
            {
                let Ok(metadata) = child.metadata() else {
                    continue;
                };
                directory.videos.push(LibraryEntry {
                    name: child.file_name().to_string_lossy().into_owned(),
                    path,
                    engine_mode: false,
                    wallpaper_type: WallpaperType::Video,
                    preview_path: None,
                    size_bytes: metadata.len(),
                    modified_unix_seconds: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_secs()),
                    scene_compatibility: None,
                    scene_properties: Vec::new(),
                });
            }
        }
        directory.children.sort();
        directory.videos.sort_by(|a, b| a.path.cmp(&b.path));
        self.directories
            .insert(path.to_path_buf(), directory.clone());
        directory
    }
}

fn related(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

struct Monitor {
    inotify: Option<Inotify>,
    watches: HashMap<WatchDescriptor, PathBuf>,
    paths: HashMap<PathBuf, WatchDescriptor>,
    used: HashSet<PathBuf>,
    targets: HashSet<PathBuf>,
    reliable: bool,
}

impl Monitor {
    fn new() -> Self {
        let inotify = match Inotify::init() {
            Ok(inotify) => Some(inotify),
            Err(error) => {
                warn!(%error, "library change notifications unavailable; falling back to scanning");
                None
            }
        };
        let reliable = inotify.is_some();
        Self {
            inotify,
            watches: HashMap::new(),
            paths: HashMap::new(),
            used: HashSet::new(),
            targets: HashSet::new(),
            reliable,
        }
    }

    fn begin_scan(&mut self) {
        self.used.clear();
        self.targets.clear();
    }

    fn track(&mut self, path: &Path) {
        // Watch ancestors as well: absent roots and nested files, directory
        // replacements and removable library mounts must remain discoverable.
        self.targets.insert(path.to_path_buf());
        let mut suffix = PathBuf::new();
        let resolved = path.canonicalize().ok();
        for ancestor in path.ancestors() {
            if let Ok(canonical) = ancestor.canonicalize() {
                self.targets.insert(canonical.join(&suffix));
                break;
            }
            if let Some(name) = ancestor.file_name() {
                suffix = PathBuf::from(name).join(suffix);
            }
        }
        for ancestor in path
            .ancestors()
            .chain(resolved.iter().flat_map(|path| path.ancestors()))
        {
            let Ok(canonical) = ancestor.canonicalize() else {
                continue;
            };
            let ancestor = canonical.as_path();
            if !ancestor.is_dir() {
                continue;
            }
            self.used.insert(ancestor.to_path_buf());
            if self.paths.contains_key(ancestor) {
                continue;
            }
            let Some(inotify) = &mut self.inotify else {
                continue;
            };
            let mask = WatchMask::CREATE
                | WatchMask::DELETE
                | WatchMask::MOVED_FROM
                | WatchMask::MOVED_TO
                | WatchMask::CLOSE_WRITE
                | WatchMask::MODIFY
                | WatchMask::ATTRIB
                | WatchMask::DELETE_SELF
                | WatchMask::MOVE_SELF
                | WatchMask::ONLYDIR;
            match inotify.watches().add(ancestor, mask) {
                Ok(watch) => {
                    // Different Steam aliases can refer to the same inode.
                    if let Some(old_path) =
                        self.watches.insert(watch.clone(), ancestor.to_path_buf())
                    {
                        self.paths.remove(&old_path);
                    }
                    self.paths.insert(ancestor.to_path_buf(), watch);
                }
                Err(error) => {
                    if self.reliable {
                        warn!(path = %ancestor.display(), %error, "library watch failed; falling back to scanning");
                    }
                    self.reliable = false;
                }
            }
        }
    }

    fn end_scan(&mut self) {
        let obsolete: Vec<_> = self
            .paths
            .keys()
            .filter(|path| !self.used.contains(*path))
            .cloned()
            .collect();
        for path in obsolete {
            if let Some(watch) = self.paths.remove(&path) {
                self.watches.remove(&watch);
                if let Some(inotify) = &mut self.inotify {
                    let _ = inotify.watches().remove(watch);
                }
            }
        }
    }

    fn changes(&mut self) -> Option<HashSet<PathBuf>> {
        let inotify = self.inotify.as_mut()?;
        let mut changed = HashSet::new();
        let mut buffer = [0; 65536];
        loop {
            match inotify.read_events(&mut buffer) {
                Ok(events) => {
                    let mut count = 0;
                    for event in events {
                        count += 1;
                        if event.mask.contains(EventMask::Q_OVERFLOW) {
                            warn!("Library notification queue overflowed; rebuilding index");
                            *self = Self::new();
                            return None;
                        }
                        if let Some(directory) = self.watches.get(&event.wd) {
                            changed.insert(
                                event
                                    .name
                                    .map_or_else(|| directory.clone(), |name| directory.join(name)),
                            );
                        }
                        if event.mask.contains(EventMask::IGNORED)
                            && let Some(path) = self.watches.remove(&event.wd)
                        {
                            self.paths.remove(&path);
                        }
                    }
                    if count == 0 {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    warn!(%error, "failed to read library notifications; rebuilding index");
                    *self = Self::new();
                    return None;
                }
            }
        }
        self.reliable.then_some(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn refresh(index: &mut LibraryIndex, home: &Path, root: &Path) -> Arc<LibraryPayload> {
        index.refresh(vec![root.to_path_buf()], home)
    }

    #[test]
    fn unchanged_requests_reuse_snapshot_and_changes_only_read_affected_directory() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("videos");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/first.mp4"), b"one").unwrap();
        fs::write(root.join("b/second.mp4"), b"two").unwrap();
        let mut index = LibraryIndex::new();
        let first = refresh(&mut index, home.path(), &root);
        assert_eq!(first.entries.len(), 2);
        assert_eq!(index.directory_reads, 3);
        for _ in 0..10 {
            assert!(Arc::ptr_eq(
                &first,
                &refresh(&mut index, home.path(), &root)
            ));
        }
        assert_eq!(index.directory_reads, 3);

        fs::write(root.join("a/new.webm"), b"new").unwrap();
        let added = refresh(&mut index, home.path(), &root);
        assert_eq!(added.entries.len(), 3);
        assert_eq!(index.directory_reads, 4);
        fs::rename(root.join("a/new.webm"), root.join("b/moved.webm")).unwrap();
        let renamed = refresh(&mut index, home.path(), &root);
        assert!(renamed.entry(&root.join("a/new.webm")).is_none());
        assert!(renamed.entry(&root.join("b/moved.webm")).is_some());
        assert_eq!(index.directory_reads, 6);
        fs::remove_dir_all(root.join("a")).unwrap();
        let removed = refresh(&mut index, home.path(), &root);
        assert_eq!(removed.entries.len(), 2);
        assert!(!index.directories.contains_key(&root.join("a")));
        assert_eq!(index.directory_reads, 7);
        fs::write(root.join("b/second.mp4"), b"updated video").unwrap();
        let modified = refresh(&mut index, home.path(), &root);
        assert_eq!(
            modified
                .entry(&root.join("b/second.mp4"))
                .unwrap()
                .size_bytes,
            13
        );
        assert_eq!(index.directory_reads, 8);
    }

    #[test]
    fn discovers_missing_roots_and_subtrees_and_applies_root_configuration_changes() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("missing/videos");
        let mut index = LibraryIndex::new();
        assert!(refresh(&mut index, home.path(), &root).entries.is_empty());
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/clip.mp4"), b"video").unwrap();
        assert_eq!(refresh(&mut index, home.path(), &root).entries.len(), 1);
        fs::rename(&root, root.with_file_name("old")).unwrap();
        assert!(refresh(&mut index, home.path(), &root).entries.is_empty());
        fs::create_dir(&root).unwrap();
        fs::write(root.join("new.mp4"), b"video").unwrap();
        assert_eq!(refresh(&mut index, home.path(), &root).entries.len(), 1);
        let added = index.refresh(vec![root.clone(), root.with_file_name("old")], home.path());
        assert_eq!(added.entries.len(), 2);
        let removed = index.refresh(vec![root], home.path());
        assert_eq!(removed.entries.len(), 1);
        assert!(
            index
                .directories
                .keys()
                .all(|path| !path.starts_with(home.path().join("missing/old")))
        );
    }

    fn project(root: &Path, id: &str, title: &str) -> PathBuf {
        let project = root.join(id);
        fs::create_dir_all(project.join("nested")).unwrap();
        fs::write(project.join("nested/movie.mp4"), b"video").unwrap();
        descriptor(&project, title);
        project
    }

    fn descriptor(project: &Path, title: &str) {
        fs::write(project.join("project.json"), format!(r#"{{"type":"video","file":"nested/movie.mp4","title":"{title}","preview":"nested/preview.jpg"}}"#)).unwrap();
    }

    #[test]
    fn workshop_changes_reparse_only_changed_projects_including_nested_dependencies() {
        let home = tempfile::tempdir().unwrap();
        let root = home
            .path()
            .join(".local/share/Steam/steamapps/workshop/content/431960");
        let a = project(&root, "1", "first");
        let b = project(&root, "2", "second");
        let mut index = LibraryIndex::new();
        let first = index.refresh(vec![], home.path());
        assert_eq!(first.entries.len(), 2);
        assert_eq!(index.project_reads, 2);
        assert!(Arc::ptr_eq(&first, &index.refresh(vec![], home.path())));
        fs::write(a.join("nested/preview.jpg"), b"image").unwrap();
        let preview = index.refresh(vec![], home.path());
        assert_eq!(
            preview.entries[0].preview_path,
            Some(a.join("nested/preview.jpg"))
        );
        assert_eq!(index.project_reads, 3);
        assert_eq!(index.directory_reads, 1);
        descriptor(&a, "renamed");
        let renamed = index.refresh(vec![], home.path());
        assert_eq!(
            renamed.entry(&a.join("nested/movie.mp4")).unwrap().name,
            "renamed"
        );
        assert_eq!(index.project_reads, 4);
        fs::remove_file(b.join("nested/movie.mp4")).unwrap();
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 1);
        assert_eq!(index.project_reads, 5);
        fs::write(b.join("nested/movie.mp4"), b"restored").unwrap();
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 2);
        assert_eq!(index.project_reads, 6);
        project(&root, "3", "third");
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 3);
        assert_eq!(index.project_reads, 7);
        fs::remove_dir_all(&a).unwrap();
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 2);
        assert_eq!(index.project_reads, 7);
    }

    #[test]
    fn discovers_new_steam_libraries_and_handles_symlink_aliases() {
        let home = tempfile::tempdir().unwrap();
        let steam = home.path().join(".local/share/Steam");
        let mut index = LibraryIndex::new();
        assert!(index.refresh(vec![], home.path()).entries.is_empty());
        let root = steam.join("steamapps/workshop/content/431960");
        project(&root, "1", "default library");
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 1);
        fs::create_dir(home.path().join(".steam")).unwrap();
        symlink(&steam, home.path().join(".steam/steam")).unwrap();
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 1);
        let external = home.path().join("external");
        let external_root = external.join("steamapps/workshop/content/431960");
        project(&external_root, "2", "external library");
        fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!("\"path\" \"{}\"", external.display()),
        )
        .unwrap();
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 2);
        let reads = index.project_reads;
        descriptor(&root.join("1"), "updated through alias");
        let updated = index.refresh(vec![], home.path());
        assert!(
            updated
                .entries
                .iter()
                .any(|entry| entry.name == "updated through alias")
        );
        assert_eq!(index.project_reads, reads + 1);
        fs::remove_file(steam.join("steamapps/libraryfolders.vdf")).unwrap();
        assert_eq!(index.refresh(vec![], home.path()).entries.len(), 1);
    }

    #[test]
    fn preserves_depth_limit_entry_limit_symlink_boundaries_and_fallback_freshness() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("videos");
        let deep = root.join("a/b/c/d/e");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("too-deep.mp4"), b"video").unwrap();
        fs::write(deep.parent().unwrap().join("at-limit.mp4"), b"video").unwrap();
        let outside = home.path().join("outside.mp4");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, root.join("link.mp4")).unwrap();
        let mut index = LibraryIndex::new();
        assert_eq!(refresh(&mut index, home.path(), &root).entries.len(), 1);
        for number in 0..MAX_LIBRARY_ENTRIES + 1 {
            fs::write(root.join(format!("{number}.mp4")), b"video").unwrap();
        }
        let capped = refresh(&mut index, home.path(), &root);
        assert_eq!(capped.entries.len(), MAX_LIBRARY_ENTRIES);
        assert!(capped.truncated);
        assert!(capped.entry(&root.join("link.mp4")).is_none());
        assert!(capped.entry(&outside).is_none());
        index.monitor.reliable = false;
        fs::remove_file(root.join("0.mp4")).unwrap();
        let fallback = refresh(&mut index, home.path(), &root);
        assert!(fallback.entry(&root.join("0.mp4")).is_none());
        assert!(!Arc::ptr_eq(&capped, &fallback));
    }

    #[test]
    fn list_thumbnail_and_media_requests_share_index_and_keep_file_boundaries() {
        use better_wallpaper_core::desktop::DesktopKind;

        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("videos");
        fs::create_dir(&root).unwrap();
        let video = root.join("clip.mp4");
        fs::write(&video, b"video").unwrap();
        let workshop = home
            .path()
            .join(".local/share/Steam/steamapps/workshop/content/431960/1");
        fs::create_dir_all(&workshop).unwrap();
        let web = workshop.join("index.html");
        let preview = workshop.join("preview.jpg");
        fs::write(&web, b"html").unwrap();
        fs::write(&preview, b"image").unwrap();
        fs::write(
            workshop.join("project.json"),
            r#"{"type":"web","file":"index.html","preview":"preview.jpg"}"#,
        )
        .unwrap();
        let mut config = AppConfig::default();
        config.library.paths = vec![root];
        let state = ApiState::new(
            Arc::new(RwLock::new(config)),
            ConfigStore::new(home.path().join("config.toml"), home.path().into()),
            DesktopDetection {
                kind: DesktopKind::Unknown,
                evidence: "test".into(),
                candidates: vec![],
            },
            BackendKind::Headless,
            PlaybackControl::default(),
            home.path().into(),
            LogStore::new(10),
            LogLevelController::new("info"),
        );
        assert_eq!(library_response(&state).status_code(), StatusCode(200));
        let snapshot = library_snapshot(&state);
        for _ in 0..5 {
            let request = tiny_http::TestRequest::new().into();
            assert_eq!(
                media_response(&request, video.clone(), &state, true).status_code(),
                StatusCode(200)
            );
            assert_eq!(
                library_thumbnail_response(
                    &format!("/api/v1/library/thumbnail?path={}", web.display()),
                    &state
                )
                .status_code(),
                StatusCode(200)
            );
            assert!(Arc::ptr_eq(&snapshot, &library_snapshot(&state)));
        }
        assert_eq!(state.library_index.lock().unwrap().directory_reads, 2);
        assert_eq!(state.library_index.lock().unwrap().project_reads, 1);

        // Simulate a filesystem replacement before notification processing. A
        // cached entry must never authorize following its path outside the library.
        let secret = home.path().join("secret.mp4");
        fs::write(&secret, b"secret").unwrap();
        fs::remove_file(&video).unwrap();
        symlink(&secret, &video).unwrap();
        fs::remove_file(&preview).unwrap();
        symlink(&secret, &preview).unwrap();
        let _ = state.library_index.lock().unwrap().monitor.changes();
        let request = tiny_http::TestRequest::new().into();
        assert_eq!(
            media_response(&request, video, &state, true).status_code(),
            StatusCode(403)
        );
        assert_eq!(
            library_thumbnail_response(
                &format!("/api/v1/library/thumbnail?path={}", web.display()),
                &state
            )
            .status_code(),
            StatusCode(403)
        );
    }

    #[test]
    fn follows_configured_root_and_safe_media_alias_changes() {
        let home = tempfile::tempdir().unwrap();
        let videos = home.path().join("videos");
        fs::create_dir(&videos).unwrap();
        fs::write(videos.join("a.mp4"), b"video").unwrap();
        let alias = home.path().join("alias");
        symlink(&videos, &alias).unwrap();
        let mut index = LibraryIndex::new();
        assert_eq!(refresh(&mut index, home.path(), &alias).entries.len(), 1);
        fs::remove_file(&alias).unwrap();
        let other = home.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("b.mp4"), b"new video").unwrap();
        symlink(&other, &alias).unwrap();
        let replaced = refresh(&mut index, home.path(), &alias);
        assert_eq!(replaced.entries[0].path, other.join("b.mp4"));

        let root = home
            .path()
            .join(".local/share/Steam/steamapps/workshop/content/431960");
        let project = project(&root, "1", "alias movie");
        fs::create_dir(project.join("media")).unwrap();
        let target = project.join("media/actual.mp4");
        fs::write(&target, b"actual").unwrap();
        fs::remove_file(project.join("nested/movie.mp4")).unwrap();
        symlink(&target, project.join("nested/movie.mp4")).unwrap();
        index.refresh(vec![], home.path());
        fs::write(&target, b"updated actual").unwrap();
        let updated = index.refresh(vec![], home.path());
        assert_eq!(updated.entry(&target).unwrap().size_bytes, 14);
    }
}
