//! Runtime-compatibility analyzer.

use super::layer::{get_file_content_from_layer, read_files_from_layer};
use super::oci_image::{final_filesystem, FinalFile, ImageConfig};
use super::{FileTreeItem, ImageLayer, Op};
use crate::store::CacheStore;
use goblin::container::Ctx;
use goblin::elf::section_header::{SectionHeader, SHT_NOBITS};
use goblin::elf::Elf;
use goblin::strtab::Strtab;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

const MIN_STRIP_CANDIDATE_BYTES: u64 = 1024 * 1024;
const MAX_STRIP_CANDIDATES: usize = 40;
const MAX_STRIP_CANDIDATE_BYTES: u64 = 512 * 1024 * 1024;
const MIN_STRIP_SAVINGS: u64 = 256 * 1024;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnstrippedBinary {
    pub layer_index: usize,
    pub path: String,
    pub size: u64,
    pub debug_bytes: u64,
    pub symbol_bytes: u64,
}

impl UnstrippedBinary {
    pub fn savings(&self) -> u64 {
        self.debug_bytes + self.symbol_bytes
    }
}

fn is_executable(mode: &str) -> bool {
    let c: Vec<char> = mode.chars().collect();
    c.len() >= 10 && [3, 6, 9].iter().any(|&i| matches!(c[i], 'x' | 's' | 't'))
}

fn is_shared_object(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".so") || name.contains(".so.")
}

fn strip_savings(bytes: &[u8]) -> Option<(u64, u64)> {
    let header = Elf::parse_header(bytes).ok()?;
    let ctx = Ctx {
        container: header.container().ok()?,
        le: header.endianness().ok()?,
    };
    let sections =
        SectionHeader::parse(bytes, header.e_shoff as usize, header.e_shnum as usize, ctx).ok()?;
    let names = sections.get(header.e_shstrndx as usize)?;
    let names = Strtab::parse(bytes, names.sh_offset as usize, names.sh_size as usize, 0).ok()?;
    let mut debug = 0;
    let mut symbols = 0;
    for section in sections.iter().filter(|s| s.sh_type != SHT_NOBITS) {
        let name = names.get_at(section.sh_name).unwrap_or_default();
        if name.starts_with(".debug_") || name.starts_with(".zdebug_") {
            debug += section.sh_size;
        } else if name == ".symtab" || name == ".strtab" {
            symbols += section.sh_size;
        }
    }
    Some((debug, symbols))
}

pub(crate) fn find_unstripped_binaries(
    layers: &[ImageLayer],
    file_tree_list: &[Vec<FileTreeItem>],
    store: &CacheStore,
    layer_concurrency: usize,
) -> Vec<UnstrippedBinary> {
    let mut candidates: Vec<(String, FinalFile)> = final_filesystem(file_tree_list)
        .into_iter()
        .filter(|(path, file)| {
            (MIN_STRIP_CANDIDATE_BYTES..=MAX_STRIP_CANDIDATE_BYTES).contains(&file.size)
                && file.link.is_empty()
                && file.mode.starts_with('-')
                && (is_executable(&file.mode) || is_shared_object(path))
        })
        .collect();
    candidates.sort_by_key(|(_, file)| Reverse(file.size));
    candidates.truncate(MAX_STRIP_CANDIDATES);

    let mut by_layer: HashMap<usize, HashSet<String>> = HashMap::new();
    for (path, file) in &candidates {
        by_layer.entry(file.layer).or_default().insert(path.clone());
    }
    let jobs: Vec<(usize, HashSet<String>)> = by_layer.into_iter().collect();
    if jobs.is_empty() {
        return vec![];
    }
    let found = Mutex::new(Vec::new());
    let next_job = AtomicUsize::new(0);
    let scan_jobs = || {
        while let Some((layer_index, paths)) = jobs.get(next_job.fetch_add(1, Ordering::Relaxed)) {
            let Some(layer) = layers.get(*layer_index) else {
                continue;
            };
            let Ok(file) = File::open(store.blob_path(&layer.digest)) else {
                continue;
            };
            let _ = read_files_from_layer(
                BufReader::new(file),
                &layer.media_type,
                paths,
                MAX_STRIP_CANDIDATE_BYTES,
                |path, bytes| {
                    let Some((debug_bytes, symbol_bytes)) = strip_savings(bytes) else {
                        return;
                    };
                    if debug_bytes + symbol_bytes >= MIN_STRIP_SAVINGS {
                        if let Ok(mut found) = found.lock() {
                            found.push(UnstrippedBinary {
                                layer_index: *layer_index,
                                path: path.to_string(),
                                size: bytes.len() as u64,
                                debug_bytes,
                                symbol_bytes,
                            });
                        }
                    }
                },
            );
        }
    };
    let workers = layer_concurrency.max(1).min(jobs.len());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(scan_jobs);
        }
    });
    let mut found = found.into_inner().unwrap_or_default();
    found.sort_by_key(|b| (Reverse(b.savings()), b.path.clone()));
    found
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCompat {
    pub entrypoint: String,
    pub libc: String,
    pub required_glibc: String,
    pub needed_libs: Vec<String>,
    pub os_glibc: String,
    pub issue: String,
}

const NEEDED_LIBS_CAP: usize = 32;
const MAX_BINARY_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

pub(crate) fn analyze_runtime_compat(
    config: &ImageConfig,
    layers: &[ImageLayer],
    file_tree_list: &[Vec<FileTreeItem>],
    base_os: &str,
    store: &CacheStore,
) -> Option<RuntimeCompat> {
    let extra = config.config.as_ref()?;
    let entrypoint = extra.entrypoint.as_deref().unwrap_or(&[]);
    let cmd = extra.cmd.as_deref().unwrap_or(&[]);

    let first = entrypoint
        .iter()
        .chain(cmd.iter())
        .find(|s| !s.is_empty())
        .cloned()?;

    let cmd_first = if entrypoint.iter().any(|s| !s.is_empty()) {
        cmd.iter().find(|s| !s.is_empty()).cloned()
    } else {
        None
    };

    let env_path = extra.env.as_ref().and_then(|envs| {
        envs.iter()
            .find_map(|e| e.strip_prefix("PATH=").map(|s| s.to_string()))
    });

    let probe1 = probe_binary(&first, env_path.as_deref(), file_tree_list, layers, store)?;
    let Probe {
        path: orig_path,
        bytes: orig_bytes,
    } = probe1;

    let (final_path, final_bytes, wrapper) = match orig_bytes {
        Some(bytes) if Elf::parse(&bytes).is_ok() => (orig_path, Some(bytes), None),
        Some(bytes) => {
            let candidate = parse_exec_target(&bytes, cmd_first.as_deref()).or_else(|| {
                cmd_first
                    .as_deref()
                    .filter(|c| *c != first.as_str())
                    .map(|s| s.to_string())
            });
            if let Some(target) = candidate {
                match probe_binary(&target, env_path.as_deref(), file_tree_list, layers, store) {
                    Some(Probe {
                        path: p2,
                        bytes: Some(b2),
                    }) => (p2, Some(b2), Some(orig_path)),
                    _ => (orig_path, Some(bytes), None),
                }
            } else {
                (orig_path, Some(bytes), None)
            }
        }
        None => (orig_path, None, None),
    };

    let display_entry = match wrapper {
        Some(w) if w != final_path => format!("{final_path} (via {w})"),
        _ => final_path,
    };

    let Some(bytes) = final_bytes else {
        return Some(RuntimeCompat {
            entrypoint: display_entry,
            ..Default::default()
        });
    };

    let info = parse_elf(&bytes).unwrap_or_default();
    let os_glibc = glibc_version_for_os(base_os);
    let os_glibc_str = os_glibc
        .map(|(a, b)| format!("{a}.{b}"))
        .unwrap_or_default();
    let issue = classify_issue(
        &info.libc,
        info.required_glibc.as_deref(),
        base_os,
        os_glibc,
    );

    Some(RuntimeCompat {
        entrypoint: display_entry,
        libc: info.libc,
        required_glibc: info.required_glibc.unwrap_or_default(),
        needed_libs: info.needed_libs,
        os_glibc: os_glibc_str,
        issue,
    })
}

struct EntryHit {
    path: String,
    layer_index: usize,
}

struct Probe {
    path: String,
    bytes: Option<Vec<u8>>,
}

fn probe_binary(
    name: &str,
    env_path: Option<&str>,
    file_tree_list: &[Vec<FileTreeItem>],
    layers: &[ImageLayer],
    store: &CacheStore,
) -> Option<Probe> {
    let mut candidates: Vec<String> = Vec::new();
    if name.starts_with('/') {
        candidates.push(name.trim_start_matches('/').to_string());
    } else {
        let path_env = env_path.unwrap_or(DEFAULT_PATH);
        for dir in path_env.split(':').filter(|s| !s.is_empty()) {
            let dir = dir.trim_start_matches('/');
            if dir.is_empty() {
                candidates.push(name.to_string());
            } else {
                candidates.push(format!("{dir}/{name}"));
            }
        }
    }
    for cand in &candidates {
        if let Some(hit) = locate_file_in_layers(cand, file_tree_list) {
            let bytes = read_binary_bytes(&hit.path, hit.layer_index, layers, store);
            return Some(Probe {
                path: hit.path,
                bytes,
            });
        }
    }
    if !name.contains('/') {
        if let Some(hit) = find_by_basename(name, file_tree_list) {
            let bytes = read_binary_bytes(&hit.path, hit.layer_index, layers, store);
            return Some(Probe {
                path: hit.path,
                bytes,
            });
        }
    }
    None
}

fn find_by_basename(basename: &str, file_tree_list: &[Vec<FileTreeItem>]) -> Option<EntryHit> {
    for (idx, tree) in file_tree_list.iter().enumerate().rev() {
        if let Some(path) = scan_tree_for_basename(tree, basename, "") {
            if let Some(hit) = locate_file_in_layers(&path, file_tree_list) {
                return Some(hit);
            }
            return Some(EntryHit {
                path,
                layer_index: idx,
            });
        }
    }
    None
}

fn scan_tree_for_basename(items: &[FileTreeItem], basename: &str, prefix: &str) -> Option<String> {
    for item in items {
        let path = if prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{}/{}", prefix, item.name)
        };
        if item.children.is_empty() {
            if item.name == basename && item.op != Op::Removed {
                return Some(path);
            }
        } else if let Some(hit) = scan_tree_for_basename(&item.children, basename, &path) {
            return Some(hit);
        }
    }
    None
}

fn parse_exec_target(script: &[u8], cmd_first: Option<&str>) -> Option<String> {
    let text = std::str::from_utf8(script).ok()?;
    for line in text.lines() {
        let Some(after) = line.trim_start().strip_prefix("exec") else {
            continue;
        };
        if !after.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(raw) = after.split_whitespace().next() else {
            continue;
        };
        let token = raw
            .trim_start_matches(['"', '\''])
            .trim_end_matches(['"', '\'']);
        if token.is_empty() {
            continue;
        }
        if token == "$@" {
            return cmd_first.map(|s| s.to_string());
        }
        return Some(token.to_string());
    }
    None
}

fn locate_file_in_layers(path: &str, file_tree_list: &[Vec<FileTreeItem>]) -> Option<EntryHit> {
    let (idx, link) = find_leaf(path, file_tree_list)?;
    if link.is_empty() {
        return Some(EntryHit {
            path: path.to_string(),
            layer_index: idx,
        });
    }
    let resolved = resolve_link(path, &link);
    let (idx2, link2) = find_leaf(&resolved, file_tree_list)?;
    if !link2.is_empty() {
        return None;
    }
    Some(EntryHit {
        path: resolved,
        layer_index: idx2.max(idx),
    })
}

fn find_leaf(path: &str, file_tree_list: &[Vec<FileTreeItem>]) -> Option<(usize, String)> {
    let path = path.trim_start_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    for (idx, tree) in file_tree_list.iter().enumerate().rev() {
        if let Some(leaf) = walk_tree(tree, &segments) {
            if leaf.op == Op::Removed {
                return None;
            }
            return Some((idx, leaf.link.clone()));
        }
    }
    None
}

fn walk_tree<'a>(items: &'a [FileTreeItem], segments: &[&str]) -> Option<&'a FileTreeItem> {
    let (head, rest) = segments.split_first()?;
    let next = items.iter().find(|i| i.name == *head)?;
    if rest.is_empty() {
        Some(next)
    } else {
        walk_tree(&next.children, rest)
    }
}

fn resolve_link(from: &str, link: &str) -> String {
    if link.starts_with('/') {
        return link.trim_start_matches('/').to_string();
    }
    let mut dir: Vec<&str> = from.trim_start_matches('/').split('/').collect();
    dir.pop();
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                dir.pop();
            }
            other => dir.push(other),
        }
    }
    dir.join("/")
}

fn read_binary_bytes(
    path: &str,
    starting_layer: usize,
    layers: &[ImageLayer],
    store: &CacheStore,
) -> Option<Vec<u8>> {
    for layer_idx in (0..=starting_layer).rev() {
        if let Some(bytes) = read_from_layer(path, layer_idx, layers, store) {
            return Some(bytes);
        }
    }
    None
}

fn read_from_layer(
    path: &str,
    layer_idx: usize,
    layers: &[ImageLayer],
    store: &CacheStore,
) -> Option<Vec<u8>> {
    let layer = layers.get(layer_idx)?;
    if layer.digest.is_empty() {
        return None;
    }
    let blob = store.blob_path(&layer.digest);
    let file = File::open(&blob).ok()?;
    let reader = BufReader::new(file);
    let bytes =
        get_file_content_from_layer(reader, &layer.media_type, path, MAX_BINARY_BYTES as u64)
            .ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(bytes)
}

#[derive(Default)]
struct ParsedElf {
    libc: String,
    required_glibc: Option<String>,
    needed_libs: Vec<String>,
}

fn parse_elf(bytes: &[u8]) -> Option<ParsedElf> {
    let elf = Elf::parse(bytes).ok()?;
    let needed: Vec<String> = elf.libraries.iter().map(|s| s.to_string()).collect();
    let libc = classify_libc(&needed);

    let required_glibc = if libc == "glibc" {
        extract_max_glibc(&elf)
    } else {
        None
    };

    let mut capped = needed;
    if capped.len() > NEEDED_LIBS_CAP {
        capped.truncate(NEEDED_LIBS_CAP);
    }

    Some(ParsedElf {
        libc,
        required_glibc,
        needed_libs: capped,
    })
}

fn classify_libc(needed: &[String]) -> String {
    let mut has_glibc = false;
    let mut has_musl = false;
    for lib in needed {
        if lib == "libc.so.6" {
            has_glibc = true;
        }
        if lib.starts_with("libc.musl-") || lib.starts_with("ld-musl-") {
            has_musl = true;
        }
    }
    match (has_glibc, has_musl) {
        (true, _) => "glibc".to_string(),
        (false, true) => "musl".to_string(),
        (false, false) if needed.is_empty() => "static".to_string(),
        _ => String::new(),
    }
}

fn extract_max_glibc(elf: &Elf) -> Option<String> {
    let verneed = elf.verneed.as_ref()?;
    let mut best: Option<(u32, u32)> = None;
    for need_file in verneed.iter() {
        for need_ver in need_file.iter() {
            let Some(name) = elf.dynstrtab.get_at(need_ver.vna_name) else {
                continue;
            };
            if let Some(v) = parse_glibc_version(name) {
                best = Some(best.map(|cur| max_tuple(cur, v)).unwrap_or(v));
            }
        }
    }
    best.map(|(a, b)| format!("{a}.{b}"))
}

fn max_tuple(a: (u32, u32), b: (u32, u32)) -> (u32, u32) {
    if a.0 > b.0 || (a.0 == b.0 && a.1 >= b.1) {
        a
    } else {
        b
    }
}

fn parse_glibc_version(s: &str) -> Option<(u32, u32)> {
    let rest = s.strip_prefix("GLIBC_")?;
    if rest == "PRIVATE" {
        return None;
    }
    let mut parts = rest.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

pub fn glibc_version_for_os(base_os: &str) -> Option<(u32, u32)> {
    let s = base_os.to_lowercase();
    if s.contains("alpine") || s.contains("musl") {
        return None;
    }
    if s.contains("debian") {
        if s.contains("trixie") || s.contains("debian 13") {
            return Some((2, 41));
        }
        if s.contains("bookworm") || s.contains("debian 12") {
            return Some((2, 36));
        }
        if s.contains("bullseye") || s.contains("debian 11") {
            return Some((2, 31));
        }
        if s.contains("buster") || s.contains("debian 10") {
            return Some((2, 28));
        }
    }
    if s.contains("ubuntu") {
        if s.contains("24.04") || s.contains("noble") {
            return Some((2, 39));
        }
        if s.contains("22.04") || s.contains("jammy") {
            return Some((2, 35));
        }
        if s.contains("20.04") || s.contains("focal") {
            return Some((2, 31));
        }
        if s.contains("18.04") || s.contains("bionic") {
            return Some((2, 27));
        }
    }
    let rhel_like = s.contains("rhel")
        || s.contains("red hat")
        || s.contains("ubi")
        || s.contains("rocky")
        || s.contains("alma")
        || s.contains("centos");
    if rhel_like {
        if s.contains('9') {
            return Some((2, 34));
        }
        if s.contains('8') {
            return Some((2, 28));
        }
        if s.contains('7') {
            return Some((2, 17));
        }
    }
    if s.contains("amazon linux 2023") || s.contains("al2023") {
        return Some((2, 34));
    }
    if s.contains("amazon linux 2") || s.contains("amzn2") {
        return Some((2, 26));
    }
    None
}

fn classify_issue(
    libc: &str,
    required_glibc: Option<&str>,
    base_os: &str,
    os_glibc: Option<(u32, u32)>,
) -> String {
    let os_lower = base_os.to_lowercase();
    let os_is_musl = os_lower.contains("alpine") || os_lower.contains("musl");

    match libc {
        "glibc" if os_is_musl => "glibc-on-musl".to_string(),
        "glibc" => {
            let (Some(req_raw), Some(os)) = (required_glibc, os_glibc) else {
                return String::new();
            };
            let Some(req) = parse_glibc_version(&format!("GLIBC_{req_raw}")) else {
                return String::new();
            };
            if req.0 > os.0 || (req.0 == os.0 && req.1 > os.1) {
                "glibc-too-old".to_string()
            } else {
                String::new()
            }
        }
        "musl" if !os_is_musl && os_glibc.is_some() => "musl-on-glibc".to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_elf(sections: &[(&str, u32, u64)]) -> Vec<u8> {
        const SHT_STRTAB: u32 = 3;
        let mut names = vec![0u8];
        let mut name_offsets = Vec::new();
        for (name, _, _) in sections.iter().chain([(".shstrtab", SHT_STRTAB, 0)].iter()) {
            name_offsets.push(names.len() as u32);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
        }
        let mut body = vec![0u8; 64];
        let mut headers = vec![(0u32, 0u32, 0u64, 0u64)];
        for (i, (_, ty, size)) in sections.iter().enumerate() {
            let offset = body.len() as u64;
            if *ty != SHT_NOBITS {
                body.resize(body.len() + *size as usize, 0xaa);
            }
            headers.push((name_offsets[i], *ty, offset, *size));
        }
        let strtab_offset = body.len() as u64;
        body.extend_from_slice(&names);
        headers.push((
            *name_offsets.last().unwrap(),
            SHT_STRTAB,
            strtab_offset,
            names.len() as u64,
        ));
        let shoff = body.len() as u64;
        for (name, ty, offset, size) in &headers {
            let mut sh = Vec::with_capacity(64);
            sh.extend_from_slice(&name.to_le_bytes());
            sh.extend_from_slice(&ty.to_le_bytes());
            sh.extend_from_slice(&[0; 16]);
            sh.extend_from_slice(&offset.to_le_bytes());
            sh.extend_from_slice(&size.to_le_bytes());
            sh.extend_from_slice(&[0; 24]);
            body.extend_from_slice(&sh);
        }
        let header = &mut body[..64];
        header[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
        header[16..18].copy_from_slice(&2u16.to_le_bytes());
        header[18..20].copy_from_slice(&62u16.to_le_bytes());
        header[20..24].copy_from_slice(&1u32.to_le_bytes());
        header[40..48].copy_from_slice(&shoff.to_le_bytes());
        header[52..54].copy_from_slice(&64u16.to_le_bytes());
        header[58..60].copy_from_slice(&64u16.to_le_bytes());
        header[60..62].copy_from_slice(&(headers.len() as u16).to_le_bytes());
        header[62..64].copy_from_slice(&((headers.len() - 1) as u16).to_le_bytes());
        body
    }

    #[test]
    fn strip_savings_counts_debug_and_symbol_sections() {
        const PROGBITS: u32 = 1;
        const SYMTAB: u32 = 2;
        const STRTAB: u32 = 3;
        let elf = tiny_elf(&[
            (".text", PROGBITS, 100),
            (".debug_info", PROGBITS, 3000),
            (".zdebug_line", PROGBITS, 1000),
            (".symtab", SYMTAB, 500),
            (".strtab", STRTAB, 200),
            (".debug_fake_bss", SHT_NOBITS, 9999),
        ]);
        assert_eq!(strip_savings(&elf), Some((4000, 700)));
        assert_eq!(
            strip_savings(&tiny_elf(&[(".text", PROGBITS, 100)])),
            Some((0, 0))
        );
        assert_eq!(strip_savings(b"#!/bin/sh\necho hi\n"), None);
    }

    #[test]
    fn strip_candidates_are_executables_and_shared_objects() {
        assert!(is_executable("-rwxr-xr-x"));
        assert!(is_executable("-rw-r--r-x"));
        assert!(!is_executable("-rw-r--r--"));
        assert!(is_shared_object("usr/lib/libfoo.so"));
        assert!(is_shared_object("usr/lib/libfoo.so.1.2.3"));
        assert!(!is_shared_object("usr/share/doc/sonar.txt"));
    }

    #[test]
    fn parses_basic_glibc_versions() {
        assert_eq!(parse_glibc_version("GLIBC_2.34"), Some((2, 34)));
        assert_eq!(parse_glibc_version("GLIBC_2.34.1"), Some((2, 34)));
        assert_eq!(parse_glibc_version("GLIBC_PRIVATE"), None);
        assert_eq!(parse_glibc_version("FOO_2.34"), None);
        assert_eq!(parse_glibc_version("GLIBC_2"), None);
    }

    #[test]
    fn classifies_libc_from_needed_list() {
        assert_eq!(
            classify_libc(&["libc.so.6".into(), "libm.so.6".into()]),
            "glibc"
        );
        assert_eq!(classify_libc(&["libc.musl-x86_64.so.1".into()]), "musl");
        assert_eq!(classify_libc(&["ld-musl-aarch64.so.1".into()]), "musl");
        assert_eq!(classify_libc(&[]), "static");
        assert_eq!(classify_libc(&["libfoo.so".into()]), "");
    }

    #[test]
    fn os_map_covers_mainstream_distros() {
        assert_eq!(glibc_version_for_os("Debian 12 (bookworm)"), Some((2, 36)));
        assert_eq!(
            glibc_version_for_os("Debian GNU/Linux 13 (trixie)"),
            Some((2, 41))
        );
        assert_eq!(
            glibc_version_for_os("Ubuntu 24.04 LTS (Noble Numbat)"),
            Some((2, 39))
        );
        assert_eq!(glibc_version_for_os("Alpine Linux v3.20"), None);
        assert_eq!(glibc_version_for_os("Red Hat UBI 9"), Some((2, 34)));
        assert_eq!(glibc_version_for_os("Amazon Linux 2023"), Some((2, 34)));
        assert_eq!(glibc_version_for_os("Scratch / Distroless"), None);
    }

    #[test]
    fn issue_classification() {
        assert_eq!(
            classify_issue("glibc", Some("2.34"), "Alpine Linux v3.20", None),
            "glibc-on-musl"
        );
        assert_eq!(
            classify_issue("glibc", Some("2.34"), "Debian 12", Some((2, 36))),
            ""
        );
        assert_eq!(
            classify_issue("glibc", Some("2.39"), "Debian 12", Some((2, 36))),
            "glibc-too-old"
        );
        assert_eq!(
            classify_issue("musl", None, "Debian 12", Some((2, 36))),
            "musl-on-glibc"
        );
        assert_eq!(
            classify_issue("static", None, "Debian 12", Some((2, 36))),
            ""
        );
        assert_eq!(classify_issue("", None, "Debian 12", Some((2, 36))), "");
    }

    #[test]
    fn parse_exec_only_matches_exec_as_the_first_word() {
        let s = b"executable=1\n# exec commented\necho exec /not/first\n\texec  /bin/real --x\r\n";
        assert_eq!(parse_exec_target(s, None), Some("/bin/real".to_string()));
        let s = b"exec\nexec \"\"\nexec /after\n";
        assert_eq!(parse_exec_target(s, None), Some("/after".to_string()));
    }

    #[test]
    fn parse_exec_extracts_absolute_path() {
        let s = b"#!/bin/sh\nset -e\nexec /usr/local/bin/myapp --port 8080\n";
        assert_eq!(
            parse_exec_target(s, None),
            Some("/usr/local/bin/myapp".to_string())
        );
    }

    #[test]
    fn parse_exec_extracts_relative_name() {
        let s = b"#!/bin/sh\nexec myapp\n";
        assert_eq!(parse_exec_target(s, None), Some("myapp".to_string()));
    }

    #[test]
    fn parse_exec_dollar_at_falls_through_to_cmd() {
        let s = b"#!/bin/sh\nset -e\nexec \"$@\"\n";
        assert_eq!(
            parse_exec_target(s, Some("static-serve")),
            Some("static-serve".to_string())
        );
        assert_eq!(parse_exec_target(s, None), None);
    }

    #[test]
    fn parse_exec_returns_none_when_no_exec_line() {
        let s = b"#!/bin/sh\necho hello\n/usr/bin/foo &\n";
        assert_eq!(parse_exec_target(s, Some("ignored")), None);
    }

    #[test]
    fn parse_exec_strips_surrounding_quotes() {
        let s = b"exec '/opt/app/bin'\n";
        assert_eq!(parse_exec_target(s, None), Some("/opt/app/bin".to_string()));
        let s = b"exec \"/opt/app/bin\" --flag\n";
        assert_eq!(parse_exec_target(s, None), Some("/opt/app/bin".to_string()));
    }

    #[test]
    fn parse_exec_rejects_non_utf8() {
        let s: &[u8] = &[0xff, 0xfe, 0xfd];
        assert_eq!(parse_exec_target(s, None), None);
    }

    #[test]
    fn parse_exec_first_match_wins() {
        let s = b"exec 3<somefile\nexec /usr/local/bin/app\n";
        assert_eq!(parse_exec_target(s, None), Some("3<somefile".to_string()));
    }

    #[test]
    fn basename_scan_finds_file_at_nonstandard_path() {
        let tree = [FileTreeItem {
            name: "diving".to_string(),
            op: Op::None,
            ..Default::default()
        }];
        let hit = scan_tree_for_basename(&tree, "diving", "");
        assert_eq!(hit.as_deref(), Some("diving"));
    }

    #[test]
    fn basename_scan_descends_into_dirs() {
        let leaf = FileTreeItem {
            name: "static-serve".to_string(),
            op: Op::None,
            ..Default::default()
        };
        let bin = FileTreeItem {
            name: "bin".to_string(),
            children: vec![leaf],
            ..Default::default()
        };
        let app = FileTreeItem {
            name: "app".to_string(),
            children: vec![bin],
            ..Default::default()
        };
        let hit = scan_tree_for_basename(&[app], "static-serve", "");
        assert_eq!(hit.as_deref(), Some("app/bin/static-serve"));
    }

    #[test]
    fn basename_scan_skips_whiteouts() {
        let tree = vec![FileTreeItem {
            name: "static-serve".to_string(),
            op: Op::Removed,
            ..Default::default()
        }];
        assert_eq!(scan_tree_for_basename(&tree, "static-serve", ""), None);
    }

    #[test]
    fn resolve_link_handles_relative_and_absolute() {
        assert_eq!(
            resolve_link("usr/bin/foo", "../share/foo/foo.real"),
            "usr/share/foo/foo.real"
        );
        assert_eq!(resolve_link("usr/bin/foo", "/opt/app/foo"), "opt/app/foo");
        assert_eq!(resolve_link("usr/bin/foo", "./bar"), "usr/bin/bar");
    }
}
