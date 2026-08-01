use std::path::Path;

use lunascope_core::{
    CitationAnchor, Course, CourseConcept, CourseSource, CourseSourceKind, CourseThreadBinding,
    LunaProject, ModelReplyLanguage, NoteProvenance, NoteRevision, NoteSection, ProjectFolder,
    ReviewItem, SyllabusRevision, SyllabusStructure, UiLanguage, UltraNoteMode, UserPreferences,
};
use lunascope_storage::SqliteEventStore;

fn course(id: &str) -> Course {
    Course {
        course_id: id.to_owned(),
        title: format!("Course {id}"),
        code: None,
        mode: UltraNoteMode::Full,
        current_syllabus_revision: None,
        created_at: "2026-07-27T00:00:00Z".to_owned(),
        updated_at: "2026-07-27T00:00:00Z".to_owned(),
    }
}

fn binding(course_id: &str, thread_id: &str) -> CourseThreadBinding {
    CourseThreadBinding {
        thread_id: thread_id.to_owned(),
        course_id: course_id.to_owned(),
        bound_at: "2026-07-27T00:00:00Z".to_owned(),
    }
}

fn source(course_id: &str, source_id: &str, kind: CourseSourceKind) -> CourseSource {
    CourseSource {
        source_id: source_id.to_owned(),
        course_id: course_id.to_owned(),
        kind,
        display_name: format!("Source {source_id}"),
        content_sha256: "abc".to_owned(),
        imported_at: "2026-07-27T00:00:00Z".to_owned(),
    }
}

fn syllabus(course_id: &str, source_id: &str, revision: u32) -> SyllabusRevision {
    SyllabusRevision {
        revision_id: format!("syllabus-{course_id}-{revision}"),
        course_id: course_id.to_owned(),
        source_id: source_id.to_owned(),
        revision,
        structure: SyllabusStructure {
            course_information: vec![],
            learning_objectives: vec![format!("Objective revision {revision}")],
            meeting_times: vec![],
            topic_schedule: vec![],
            assessments: vec![],
            grading: vec![],
            textbooks: vec![],
            late_policy: vec![],
            attendance_policy: vec![],
            ai_policy: vec![],
            academic_integrity_policy: vec![],
        },
        ambiguities: vec![],
        confirmed: false,
        prestudy_plan: vec![],
        first_phase_plan: vec![],
        created_at: format!("2026-07-27T00:00:0{revision}Z"),
    }
}

fn note(course_id: &str, thread_id: &str, source_id: &str) -> NoteRevision {
    let anchor = CitationAnchor {
        anchor_id: format!("anchor-{course_id}"),
        course_id: course_id.to_owned(),
        source_id: source_id.to_owned(),
        page: Some(2),
        slide: None,
        timestamp: None,
        text_range: Some("lines 1-2".to_owned()),
        excerpt: "A cited statement".to_owned(),
    };
    NoteRevision {
        note_revision_id: format!("note-{course_id}"),
        course_id: course_id.to_owned(),
        thread_id: thread_id.to_owned(),
        source_id: source_id.to_owned(),
        revision: 1,
        title: "Lecture 1".to_owned(),
        user_note_spec: Some("Prefer concise notes.".to_owned()),
        sections: vec![NoteSection {
            heading: "Core concepts".to_owned(),
            content: vec!["A cited statement".to_owned()],
            provenance: NoteProvenance::FromClass,
            citation_anchor_ids: vec![anchor.anchor_id.clone()],
        }],
        source_map: vec![anchor],
        created_at: "2026-07-27T00:00:00Z".to_owned(),
    }
}

fn seed_course(store: &SqliteEventStore, course_id: &str, thread_id: &str, source_id: &str) {
    store.save_course(&course(course_id)).unwrap();
    store
        .bind_course_thread(&binding(course_id, thread_id))
        .unwrap();
    store
        .save_course_source(
            &source(course_id, source_id, CourseSourceKind::Syllabus),
            "Objective: understand isolation",
        )
        .unwrap();
}

#[test]
fn course_threads_and_memory_are_isolated() {
    let store = SqliteEventStore::open_in_memory().unwrap();
    seed_course(&store, "course-a", "thread-a", "source-a");
    seed_course(&store, "course-b", "thread-b", "source-b");

    assert_eq!(
        store
            .course_thread_binding("thread-a")
            .unwrap()
            .unwrap()
            .course_id,
        "course-a"
    );
    assert_eq!(
        store
            .course_thread_binding("thread-b")
            .unwrap()
            .unwrap()
            .course_id,
        "course-b"
    );

    store
        .save_course_concept(&CourseConcept {
            concept_id: "concept-a".to_owned(),
            course_id: "course-a".to_owned(),
            term: "Isolation".to_owned(),
            explanation: "Course A only".to_owned(),
            confidence: 3,
            source_anchor_ids: vec![],
        })
        .unwrap();
    assert_eq!(store.course_concepts("course-a").unwrap().len(), 1);
    assert!(store.course_concepts("course-b").unwrap().is_empty());
    let hits = store
        .search_course_sources("course-a", "isolation", 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].course_id, "course-a");
    assert!(
        store
            .search_course_sources("course-b", "missing-term", 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn syllabus_revisions_are_ordered_and_objectives_migrate_atomically() {
    let store = SqliteEventStore::open_in_memory().unwrap();
    seed_course(&store, "course-a", "thread-a", "source-a");
    store
        .save_syllabus_revision(&syllabus("course-a", "source-a", 1))
        .unwrap();
    store
        .save_syllabus_revision(&syllabus("course-a", "source-a", 2))
        .unwrap();

    let latest = store.latest_syllabus_revision("course-a").unwrap().unwrap();
    assert_eq!(latest.revision, 2);
    assert_eq!(
        latest.structure.learning_objectives,
        vec!["Objective revision 2"]
    );
}

#[test]
fn composite_foreign_keys_reject_cross_course_notes() {
    let store = SqliteEventStore::open_in_memory().unwrap();
    seed_course(&store, "course-a", "thread-a", "source-a");
    seed_course(&store, "course-b", "thread-b", "source-b");

    let error = store
        .save_note_revision(&note("course-b", "thread-b", "source-a"))
        .expect_err("source from another course must fail");
    assert!(error.to_string().contains("FOREIGN KEY constraint failed"));
    assert!(
        store
            .latest_course_notes("course-b", 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn notes_citation_anchors_and_reviews_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lunascope.db");
    {
        let store = SqliteEventStore::open(&path).unwrap();
        seed_course(&store, "course-a", "thread-a", "source-a");
        store
            .save_note_revision(&note("course-a", "thread-a", "source-a"))
            .unwrap();
        store
            .save_course_concept(&CourseConcept {
                concept_id: "concept-a".to_owned(),
                course_id: "course-a".to_owned(),
                term: "Isolation".to_owned(),
                explanation: "State is scoped to a course.".to_owned(),
                confidence: 3,
                source_anchor_ids: vec!["anchor-course-a".to_owned()],
            })
            .unwrap();
        store
            .save_review_item(&ReviewItem {
                review_item_id: "review-a".to_owned(),
                course_id: "course-a".to_owned(),
                concept_id: "concept-a".to_owned(),
                prompt: "Define isolation.".to_owned(),
                answer: "State does not leak.".to_owned(),
                interval_days: 1,
                next_review_at: "2026-07-28T00:00:00Z".to_owned(),
                confidence: 3,
                successful_reviews: 1,
                lapses: 0,
                source_anchor_ids: vec!["anchor-course-a".to_owned()],
            })
            .unwrap();
    }

    let reopened = SqliteEventStore::open(Path::new(&path)).unwrap();
    let notes = reopened.latest_course_notes("course-a", 10).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].source_map[0].page, Some(2));
    assert_eq!(reopened.course_review_items("course-a").unwrap().len(), 1);
}

#[test]
fn projects_and_preferences_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.db");
    let project = LunaProject {
        project_id: "project-a".to_owned(),
        name: "Tiny project".to_owned(),
        folders: vec![
            ProjectFolder {
                folder_id: "folder-a".to_owned(),
                path: r"D:\tiny".to_owned(),
                display_name: "tiny".to_owned(),
                is_workspace: true,
            },
            ProjectFolder {
                folder_id: "folder-b".to_owned(),
                path: r"D:\references".to_owned(),
                display_name: "references".to_owned(),
                is_workspace: false,
            },
        ],
        kind: lunascope_core::ProjectKind::General,
        ultranote: None,
        created_at: "2026-07-27T00:00:00Z".to_owned(),
        updated_at: "2026-07-27T00:00:00Z".to_owned(),
    };
    {
        let store = SqliteEventStore::open(&path).unwrap();
        store.save_project(&project).unwrap();
        store
            .save_user_preferences(&UserPreferences {
                language: UiLanguage::English,
                model_reply_language: ModelReplyLanguage::Chinese,
                ultranote_note_spec: "Use Cornell notes.".to_owned(),
            })
            .unwrap();
    }
    let store = SqliteEventStore::open(&path).unwrap();
    assert_eq!(store.projects().unwrap(), vec![project]);
    assert_eq!(
        store.user_preferences().unwrap(),
        UserPreferences {
            language: UiLanguage::English,
            model_reply_language: ModelReplyLanguage::Chinese,
            ultranote_note_spec: "Use Cornell notes.".to_owned(),
        }
    );
}
