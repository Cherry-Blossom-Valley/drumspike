use super::*;

fn skill_folder(root: &Path, version_line: &str) -> PathBuf {
    let src = root.join("bundle");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(
        src.join("SKILL.md"),
        format!("---\nname: drumspike-finetune\nversion: {version_line}\n---\nRun \"<binary>\" bridge and \"<binary>\" mcp.\n"),
    )
    .unwrap();
    std::fs::write(src.join("README.md"), "for a person <binary>\n").unwrap();
    std::fs::write(src.join("sub").join("note.txt"), "x").unwrap();
    src
}

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ds_skill_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn install_copies_the_folder_bakes_the_binary_path_in_and_counts_the_files() {
    let r = root("install");
    let src = skill_folder(&r, "<version>");
    let target = r.join("skills").join("drumspike-finetune");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let bin = Path::new("/Applications/Drum Spike.app/Contents/MacOS/drumspike-desktop");
    let out = install(&src, &target, bin, "0.9.26").unwrap();
    assert_eq!(out["files"], 3);
    assert_eq!(out["version"], "0.9.26");
    assert_eq!(out["installed"], target.display().to_string());
    let skill = std::fs::read_to_string(target.join("SKILL.md")).unwrap();
    assert!(!skill.contains(BINARY_TAG) && !skill.contains(VERSION_TAG), "{skill}");
    assert!(skill.contains("\"/Applications/Drum Spike.app/Contents/MacOS/drumspike-desktop\" bridge"));
    assert!(target.join("sub").join("note.txt").is_file());
    // the README is a file for a person: copied as it is
    assert!(
        std::fs::read_to_string(target.join("README.md"))
            .unwrap()
            .contains(BINARY_TAG)
    );
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn a_second_install_replaces_an_older_copy_and_status_tells_the_versions() {
    let r = root("replace");
    let src = skill_folder(&r, "<version>");
    let target = r.join("drumspike-finetune");
    let bin = Path::new("/opt/ds/drumspike-desktop");
    assert_eq!(status(&target, bin, "0.9.26")["installed"], false);
    install(&src, &target, bin, "0.9.25").unwrap();
    std::fs::write(target.join("stale.txt"), "left over from the old copy").unwrap();
    let s = status(&target, bin, "0.9.26");
    assert_eq!(
        (s["installed"].clone(), s["version"].clone(), s["current"].clone()),
        (json!(true), json!("0.9.25"), json!(false))
    );
    assert_eq!(s["bundled_version"], "0.9.26");
    assert_eq!(
        s["mcp_command"],
        "claude mcp add drumspike -- \"/opt/ds/drumspike-desktop\" mcp"
    );
    install(&src, &target, bin, "0.9.26").unwrap();
    assert!(
        !target.join("stale.txt").exists(),
        "the older copy is replaced, not merged"
    );
    let s = status(&target, bin, "0.9.26");
    assert_eq!(
        (s["version"].clone(), s["current"].clone()),
        (json!("0.9.26"), json!(true))
    );
    // the same version from another binary path is not current: the baked path is the old one
    assert_eq!(status(&target, Path::new("/other/bin"), "0.9.26")["current"], false);
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn install_refuses_a_folder_that_is_not_a_copy_of_the_skill_and_a_bundle_without_skill_md() {
    let r = root("refuse");
    let src = skill_folder(&r, "<version>");
    let other = r.join("documents");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("mine.txt"), "keep").unwrap();
    let e = install(&src, &other, Path::new("/b"), "1").unwrap_err();
    assert_eq!(e.code, 409);
    assert!(other.join("mine.txt").is_file());
    let empty = r.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(
        install(&empty, &r.join("t"), Path::new("/b"), "1").unwrap_err().code,
        404
    );
    assert!(
        install(&src, &r.join("fresh"), Path::new("/b"), "1").is_ok(),
        "a missing target is created"
    );
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn the_shipped_skill_folder_has_the_placeholders_the_install_writes() {
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills").join(SKILL);
    let text = std::fs::read_to_string(shipped.join("SKILL.md")).unwrap();
    assert!(text.contains(BINARY_TAG) && text.contains("version: <version>"));
    assert!(shipped.join("README.md").is_file());
    let r = root("shipped");
    let out = install(&shipped, &r.join("x"), Path::new("/b/ds"), "9.9.9").unwrap();
    let baked = std::fs::read_to_string(r.join("x").join("SKILL.md")).unwrap();
    assert!(!baked.contains(BINARY_TAG) && !baked.contains(VERSION_TAG), "{out}");
    assert_eq!(status(&r.join("x"), Path::new("/b/ds"), "9.9.9")["current"], true);
    let _ = std::fs::remove_dir_all(&r);
}
