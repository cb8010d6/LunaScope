use std::{env, fs, path::PathBuf};

use lunascope_core::{LunaProject, ProjectFolder};
use lunascope_runtime::{FilesystemTools, TextPatch, TextReplacement};
use lunascope_storage::SqliteEventStore;
use uuid::Uuid;

#[test]
#[ignore = "writes a persistent verification project under LUNASCOPE_CANARY_DATA_ROOT"]
fn creates_reads_and_modifies_a_persisted_lunascope_project() {
    let data_root = env::var_os("LUNASCOPE_CANARY_DATA_ROOT")
        .map(PathBuf::from)
        .expect("set LUNASCOPE_CANARY_DATA_ROOT to an isolated persistent canary directory");
    assert!(data_root.is_absolute(), "canary data root must be absolute");
    let verification_root = data_root.join("verification");
    let state_root = data_root.join("state");
    fs::create_dir_all(&verification_root).expect("create verification root");
    fs::create_dir_all(&state_root).expect("create state root");

    let suffix = Uuid::new_v4().simple().to_string();
    let short_suffix = &suffix[..8];
    let project_root = verification_root.join(format!("TinyLunaProject-{short_suffix}"));
    let workspace = project_root.join("workspace");
    let references = project_root.join("references");
    fs::create_dir_all(&workspace).expect("create workspace folder");
    fs::create_dir_all(&references).expect("create references folder");

    let now = jiff::Timestamp::now().to_string();
    let project = LunaProject {
        project_id: format!("project-canary-{short_suffix}"),
        name: format!("Tiny Luna Project {short_suffix}"),
        folders: vec![
            ProjectFolder {
                folder_id: format!("folder-workspace-{short_suffix}"),
                path: workspace.to_string_lossy().into_owned(),
                display_name: "workspace".to_owned(),
                is_workspace: true,
            },
            ProjectFolder {
                folder_id: format!("folder-references-{short_suffix}"),
                path: references.to_string_lossy().into_owned(),
                display_name: "references".to_owned(),
                is_workspace: false,
            },
        ],
        kind: lunascope_core::ProjectKind::General,
        ultranote: None,
        created_at: now.clone(),
        updated_at: now,
    };

    let store = SqliteEventStore::open(state_root.join("lunascope.db"))
        .expect("open the real LunaScope state database");
    store
        .save_project(&project)
        .expect("persist project metadata");

    let tools = FilesystemTools::new(&workspace).expect("scope filesystem tools to workspace");
    let created = tools
        .create_text_file(
            "README.md",
            "# Tiny Luna Project\n\nLunaScope canary status: created\n",
        )
        .expect("create project file");
    let first_read = tools
        .read_range("README.md", 0, 1024)
        .expect("read created project file");
    assert_eq!(first_read.sha256, created.sha256);
    assert!(first_read.text.contains("status: created"));

    let patched = tools
        .apply_text_patch(&TextPatch {
            path: "README.md".to_owned(),
            expected_sha256: first_read.sha256,
            replacements: vec![TextReplacement {
                old: "status: created".to_owned(),
                new: "status: verified".to_owned(),
                expected_occurrences: 1,
            }],
        })
        .expect("modify project file with guards");
    let final_read = tools
        .read_range("README.md", 0, 1024)
        .expect("read modified project file");
    assert_eq!(final_read.sha256, patched.sha256);
    assert!(final_read.text.contains("status: verified"));

    let persisted = store
        .project(&project.project_id)
        .expect("load project")
        .expect("project exists");
    assert_eq!(persisted.folders.len(), 2);
    assert_eq!(
        persisted
            .folders
            .iter()
            .filter(|folder| folder.is_workspace)
            .count(),
        1
    );

    println!("project_id={}", project.project_id);
    println!("project_root={}", project_root.display());
    println!("file={}", workspace.join("README.md").display());
    println!("sha256={}", final_read.sha256);
    println!("content={}", final_read.text.trim());
}
