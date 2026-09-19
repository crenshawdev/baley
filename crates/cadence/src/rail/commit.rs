//! A sealed single-parent commit and index CAS, installed by the store writer.
use crate::store::{Error, Result};
use super::git;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Write, path::{Path, PathBuf}, process::{Command, Stdio}};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object { pub kind: String, pub id: String, pub bytes: Vec<u8> }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seal {
    pub parent: String,
    pub reference: String,
    pub tree: String,
    pub commit: Object,
    pub objects: Vec<Object>,
    pub index_path: PathBuf,
    pub index_before: Vec<u8>,
    pub index_after: Vec<u8>,
}

fn input(root: &Path, args: &[&str], bytes: &[u8], index: Option<&Path>) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command.current_dir(root).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(index) = index { command.env("GIT_INDEX_FILE", index); }
    let mut child = command.spawn()?;
    child.stdin.take().unwrap().write_all(bytes)?;
    let output = child.wait_with_output()?;
    if !output.status.success() { return Err(Error::Io(format!("git {args:?}: {}", String::from_utf8_lossy(&output.stderr)))); }
    Ok(output.stdout)
}
fn text(root: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(git::run(root, args)?).map(|s| s.trim_end().to_owned()).map_err(|e| Error::Invalid(e.to_string()))
}
fn object(root: &Path, kind: &str, bytes: Vec<u8>) -> Result<Object> {
    let id = git::object_id(input(root, &["hash-object", "-t", kind, "--stdin"], &bytes, None)?)?;
    Ok(Object { kind: kind.into(), id, bytes })
}
fn optional_config(root: &Path, key: &str) -> Result<Option<String>> {
    let output = Command::new("git").current_dir(root).args(["config", "--get", key]).output()?;
    match output.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&output.stdout).trim_end().into())),
        Some(1) => Ok(None),
        _ => Err(Error::Policy(format!("cannot read git config {key}"))),
    }
}
fn sign(root: &Path, bytes: Vec<u8>) -> Result<Vec<u8>> {
    if !matches!(optional_config(root, "commit.gpgsign")?.as_deref(), Some("true" | "yes" | "1" | "on")) { return Ok(bytes); }
    if optional_config(root, "gpg.format")?.is_some_and(|f| f != "openpgp") {
        return Err(Error::Policy("prune committer requires OpenPGP signing".into()));
    }
    let program = optional_config(root, "gpg.program")?.unwrap_or_else(|| "gpg".into());
    let key = optional_config(root, "user.signingkey")?.ok_or_else(|| Error::Policy("prune signing needs user.signingkey".into()))?;
    let mut child = Command::new(program).current_dir(root).args(["--status-fd=2", "-bsau", &key])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(&bytes)?;
    let output = child.wait_with_output()?;
    if !output.status.success() { return Err(Error::Policy(format!("prune signing failed: {}", String::from_utf8_lossy(&output.stderr)))); }
    let signature = String::from_utf8(output.stdout).map_err(|e| Error::Invalid(e.to_string()))?;
    let source = String::from_utf8(bytes).map_err(|e| Error::Invalid(e.to_string()))?;
    let (headers, message) = source.split_once("\n\n").ok_or_else(|| Error::Invalid("invalid commit bytes".into()))?;
    Ok(format!("{headers}\ngpgsig {}\n\n{message}", signature.trim_end().replace('\n', "\n ")).into_bytes())
}

// Git's tree order treats a directory as if its name ended with '/'.
fn tree(root: &Path, entries: &BTreeMap<String, (String,String)>, prefix: &str, objects: &mut Vec<Object>) -> Result<String> {
    let mut children = BTreeMap::new();
    for (path,(mode,id)) in entries {
        let Some(rest) = path.strip_prefix(prefix) else { continue; };
        if let Some((dir,_)) = rest.split_once('/') { children.insert(format!("{dir}/"), None); }
        else { children.insert(rest.into(), Some((mode.clone(),id.clone()))); }
    }
    let mut bytes = Vec::new();
    for (name,entry) in children {
        let (name,mode,id) = if let Some((mode,id)) = entry { (name,mode,id) }
        else {
            let id = tree(root, entries, &format!("{prefix}{name}"), objects)?;
            (name.trim_end_matches('/').to_owned(),"40000".into(),id)
        };
        bytes.extend_from_slice(format!("{mode} {name}\0").as_bytes());
        for pair in id.as_bytes().chunks_exact(2) {
            bytes.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).map_err(|e| Error::Invalid(e.to_string()))?);
        }
    }
    let object = object(root,"tree",bytes)?;
    let id = object.id.clone();
    objects.push(object);
    Ok(id)
}

struct Temporary(PathBuf);
impl Drop for Temporary { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }

pub fn freeze(root: &Path, changes: &BTreeMap<String, Option<Vec<u8>>>, phases: &[u32]) -> Result<Seal> {
    let parent = git::resolve_commit(root,"HEAD")?;
    let reference = text(root,&["symbolic-ref","-q","HEAD"])?;
    if !reference.starts_with("refs/heads/") { return Err(Error::Policy("prune needs a branch ref".into())); }
    let mut entries = BTreeMap::new();
    for record in git::run(root,["ls-tree","-r","-z","--full-tree",&parent])?.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let record = std::str::from_utf8(record).map_err(|_| Error::Invalid("undecodable Git pathname".into()))?;
        let (header,path) = record.split_once('\t').ok_or_else(|| Error::Invalid("invalid Git tree row".into()))?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len()!=3 { return Err(Error::Invalid("invalid Git tree entry".into())); }
        entries.insert(path.to_owned(),(fields[0].to_owned(),fields[2].to_owned()));
    }
    for path in entries.keys() {
        if phases.iter().any(|p| path.starts_with(&format!(".planning/phases/{p}/"))) && !changes.contains_key(path) {
            return Err(Error::Conflict(format!("tracked prune input is missing: {path}")));
        }
    }
    let mut objects = Vec::new();
    let mut index_input = Vec::new();
    for (path,after) in changes {
        let (mode,_) = entries.get(path).ok_or_else(|| Error::Invalid(format!("prune input lacks Git history: {path}")))?.clone();
        if !matches!(mode.as_str(),"100644"|"100755") { return Err(Error::Invalid(format!("prune input is not a tracked regular file: {path}"))); }
        if git::run(root,["show",&format!("{parent}:{path}")])? != fs::read(root.join(path)).map_err(|e| Error::Io(format!("{path}: {e}")))? {
            return Err(Error::Conflict(format!("prune worktree differs from parent: {path}")));
        }
        let diff = git::run(root,["diff","--cached","--name-only","--no-ext-diff",&parent,"--",&format!(":(literal){path}")])?;
        if !diff.is_empty() { return Err(Error::Conflict(format!("prune index differs from parent: {path}"))); }
        if let Some(bytes) = after {
            let blob = object(root,"blob",bytes.clone())?;
            index_input.extend_from_slice(format!("{mode} {}\t{path}\0",blob.id).as_bytes());
            entries.insert(path.clone(),(mode,blob.id.clone()));
            objects.push(blob);
        } else {
            entries.remove(path);
            index_input.extend_from_slice(format!("0 {}\t{path}\0", "0".repeat(parent.len())).as_bytes());
        }
    }
    let tree = tree(root,&entries,"",&mut objects)?;
    let author = text(root,&["var","GIT_AUTHOR_IDENT"])?;
    let committer = text(root,&["var","GIT_COMMITTER_IDENT"])?;
    let commit = object(root,"commit",sign(root,format!("tree {tree}\nparent {parent}\nauthor {author}\ncommitter {committer}\n\nchore: prune milestone phases {}\n\nRetain completed phase evidence in the single parent tree.\n",phases.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")).into_bytes())?)?;
    let index_path = root.join(text(root,&["rev-parse","--git-path","index"])?);
    let index_before = fs::read(&index_path)?;
    let temporary = Temporary(index_path.with_file_name(format!(".cadence-prune-index-{}",std::process::id())));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary.0)?;
    file.write_all(&index_before)?;
    drop(file);
    input(root,&["update-index","-z","--index-info"],&index_input,Some(&temporary.0))?;
    let index_after = fs::read(&temporary.0)?;
    Ok(Seal {parent,reference,tree,commit,objects,index_path,index_before,index_after})
}

pub fn validate(root: &Path, seal: &Seal, replay: bool) -> Result<()> {
    if text(root,&["symbolic-ref","-q","HEAD"])? != seal.reference { return Err(Error::Conflict("prune branch ref changed".into())); }
    let actual = git::resolve_commit(root,&seal.reference)?;
    if actual != seal.parent && !(replay && actual == seal.commit.id) {
        return Err(Error::Conflict(format!("prune ref changed: {}",seal.reference)));
    }
    let path = root.join(text(root,&["rev-parse","--git-path","index"])?);
    if path != seal.index_path { return Err(Error::Conflict("prune index path changed".into())); }
    let actual = fs::read(&path)?;
    if actual != seal.index_before && !(replay && actual == seal.index_after) {
        return Err(Error::Conflict(format!("prune index changed: {}",path.display())));
    }
    Ok(())
}

pub fn install(root: &Path, seal: &Seal, guard: &mut dyn FnMut() -> Result<()>) -> Result<()> {
    use crate::milestone::prune::stop;
    guard()?;
    stop("objects:before")?;
    for object in &seal.objects {
        guard()?;
        let id = git::object_id(input(root,&["hash-object","-w","-t",&object.kind,"--stdin"],&object.bytes,None)?)?;
        if id != object.id { return Err(Error::Invalid("prune object identity changed".into())); }
    }
    stop("objects:after")?;
    guard()?;
    stop("commit:before")?;
    let id = git::object_id(input(root,&["hash-object","-w","-t","commit","--stdin"],&seal.commit.bytes,None)?)?;
    if id != seal.commit.id { return Err(Error::Invalid("prune commit identity changed".into())); }
    stop("commit:after")?;
    guard()?;
    stop("ref:before")?;
    if git::resolve_commit(root,&seal.reference)? != id {
        git::run(root,["update-ref",&seal.reference,&id,&seal.parent])?;
    }
    stop("ref:after")?;
    guard()?;
    stop("index:before")?;
    if fs::read(&seal.index_path)? != seal.index_after {
        let lock = Temporary(seal.index_path.with_extension("lock"));
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&lock.0)?;
        guard()?;
        file.write_all(&seal.index_after)?;
        file.sync_all()?;
        fs::rename(&lock.0,&seal.index_path)?;
        fs::File::open(seal.index_path.parent().unwrap())?.sync_all()?;
    }
    stop("index:after")?;
    Ok(())
}
