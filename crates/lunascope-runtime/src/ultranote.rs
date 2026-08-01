use std::str::FromStr;

use jiff::{SignedDuration, Timestamp};
use lunascope_core::{
    CitationAnchor, Course, CourseAssignment, CourseConcept, CourseMemory, CourseSource,
    HomeworkAssistanceMode, HomeworkKind, HomeworkPolicyDecision, NoteProvenance, NoteRevision,
    NoteSection, ReviewItem, SyllabusRevision, SyllabusStructure, UltraNoteWorkspace,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const NOTE_HEADINGS: [&str; 12] = [
    "Metadata",
    "Learning objectives",
    "Prerequisites",
    "Core concepts",
    "Derivations and reasoning",
    "Examples",
    "Instructor emphasis",
    "Confusing or missing points",
    "Summary",
    "Retrieval practice",
    "Flashcards",
    "Homework and action items",
];

#[derive(Debug, Error)]
pub enum UltraNoteError {
    #[error("course id is required")]
    MissingCourseId,
    #[error("thread id is required")]
    MissingThreadId,
    #[error("source content is empty")]
    EmptySource,
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),
}

pub fn sha256_text(content: &str) -> String {
    Sha256::digest(content.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn parse_syllabus(
    course: &Course,
    source: &CourseSource,
    content: &str,
    revision: u32,
    now: &str,
) -> Result<SyllabusRevision, UltraNoteError> {
    validate_input(&course.course_id, None, content)?;
    let mut structure = SyllabusStructure {
        course_information: Vec::new(),
        learning_objectives: Vec::new(),
        meeting_times: Vec::new(),
        topic_schedule: Vec::new(),
        assessments: Vec::new(),
        grading: Vec::new(),
        textbooks: Vec::new(),
        late_policy: Vec::new(),
        attendance_policy: Vec::new(),
        ai_policy: Vec::new(),
        academic_integrity_policy: Vec::new(),
    };

    for line in meaningful_lines(content) {
        let lower = line.to_lowercase();
        if contains_any(&lower, &["course", "instructor", "office", "department"]) {
            structure.course_information.push(line.clone());
        }
        if contains_any(
            &lower,
            &["objective", "outcome", "you will", "students will"],
        ) {
            structure.learning_objectives.push(line.clone());
        }
        if contains_any(
            &lower,
            &[
                "meeting",
                "monday",
                "tuesday",
                "wednesday",
                "thursday",
                "friday",
                "time:",
            ],
        ) {
            structure.meeting_times.push(line.clone());
        }
        if contains_any(&lower, &["week ", "topic", "schedule", "module "]) {
            structure.topic_schedule.push(line.clone());
        }
        if contains_any(
            &lower,
            &["assignment", "homework", "exam", "quiz", "project"],
        ) {
            structure.assessments.push(line.clone());
        }
        if contains_any(&lower, &["grading", "grade", "percent", "%"]) {
            structure.grading.push(line.clone());
        }
        if contains_any(&lower, &["textbook", "reading", "isbn", "required text"]) {
            structure.textbooks.push(line.clone());
        }
        if contains_any(&lower, &["late", "extension", "deadline"]) {
            structure.late_policy.push(line.clone());
        }
        if contains_any(&lower, &["attendance", "absence", "absent"]) {
            structure.attendance_policy.push(line.clone());
        }
        if contains_any(
            &lower,
            &[
                "artificial intelligence",
                "generative ai",
                "chatgpt",
                "ai use",
                "llm",
            ],
        ) {
            structure.ai_policy.push(line.clone());
        }
        if contains_any(
            &lower,
            &[
                "academic integrity",
                "honor code",
                "plagiarism",
                "misconduct",
            ],
        ) {
            structure.academic_integrity_policy.push(line);
        }
    }
    deduplicate_structure(&mut structure);

    let mut ambiguities = Vec::new();
    push_missing(
        &mut ambiguities,
        &structure.learning_objectives,
        "Learning objectives were not located; confirm them with the instructor or syllabus.",
    );
    push_missing(
        &mut ambiguities,
        &structure.topic_schedule,
        "Topic schedule was not located; next-topic pre-study cannot be inferred safely.",
    );
    push_missing(
        &mut ambiguities,
        &structure.assessments,
        "Assessment and exam details were not located; no exam dates or requirements were inferred.",
    );
    push_missing(
        &mut ambiguities,
        &structure.ai_policy,
        "AI-use policy was not located; graded-work assistance remains restricted pending clarification.",
    );
    push_missing(
        &mut ambiguities,
        &structure.academic_integrity_policy,
        "Academic-integrity policy was not located; confirm it before graded work.",
    );

    let next_topic = structure
        .topic_schedule
        .first()
        .cloned()
        .unwrap_or_else(|| "Instructor-confirmed next topic".to_owned());
    let prestudy_plan = vec![
        format!("5 min — recall prerequisite ideas for {next_topic} without notes."),
        format!("10 min — preview definitions and headings for {next_topic}."),
        "5 min — write two questions and one prediction for the lecture.".to_owned(),
    ];
    let first_phase_plan = vec![
        "Confirm syllabus ambiguities and course policies.".to_owned(),
        "Create one independent lecture thread per class meeting or source bundle.".to_owned(),
        "Complete the first 20-minute pre-study session.".to_owned(),
    ];

    Ok(SyllabusRevision {
        revision_id: format!("syllabus-{}", Uuid::new_v4()),
        course_id: course.course_id.clone(),
        source_id: source.source_id.clone(),
        revision: revision.max(1),
        structure,
        ambiguities,
        confirmed: false,
        prestudy_plan,
        first_phase_plan,
        created_at: now.to_owned(),
    })
}

pub fn generate_lecture_notes(
    course_id: &str,
    thread_id: &str,
    source: &CourseSource,
    content: &str,
    user_note_spec: Option<&str>,
    revision: u32,
    now: &str,
) -> Result<NoteRevision, UltraNoteError> {
    validate_input(course_id, Some(thread_id), content)?;
    let lines = meaningful_lines(content);
    let source_map = build_anchors(course_id, source, &lines);
    let all_anchor_ids = source_map
        .iter()
        .map(|anchor| anchor.anchor_id.clone())
        .collect::<Vec<_>>();
    let summary = lines.iter().take(5).cloned().collect::<Vec<_>>();
    let concepts = lines
        .iter()
        .filter(|line| {
            let lower = line.to_lowercase();
            contains_any(&lower, &[" is ", " means ", "defined", "definition", ":"])
        })
        .take(8)
        .cloned()
        .collect::<Vec<_>>();

    let section_content = [
        vec![
            format!("Source: {}", source.display_name),
            format!("Source kind: {:?}", source.kind),
            format!("Imported: {}", source.imported_at),
        ],
        vec!["Confirm the learning objectives against the syllabus and instructor material.".to_owned()],
        vec!["List prerequisite concepts explicitly before the next review.".to_owned()],
        fallback(concepts, "No explicit definitions were detected; add confirmed concepts during review."),
        vec!["No derivation is attributed to the instructor unless it appears in the source map.".to_owned()],
        vec!["Add worked examples and link each one to a source anchor.".to_owned()],
        vec!["No instructor emphasis was confirmed in the imported text.".to_owned()],
        vec!["Resolve ambiguous terminology and missing context before treating it as course memory.".to_owned()],
        fallback(summary, "The source did not contain enough text for a summary."),
        vec!["Close the notes and explain the three most important ideas from memory.".to_owned()],
        vec!["Create flashcards only for stable definitions, formulas, or vocabulary.".to_owned()],
        vec!["Classify each task as practice, ungraded, graded, exam, or unknown before requesting help.".to_owned()],
    ];

    let sections = NOTE_HEADINGS
        .into_iter()
        .zip(section_content)
        .map(|(heading, content)| NoteSection {
            heading: heading.to_owned(),
            content,
            provenance: if heading == "Metadata"
                || heading == "Summary"
                || heading == "Core concepts"
            {
                NoteProvenance::FromClass
            } else if heading == "Instructor emphasis" || heading == "Confusing or missing points" {
                NoteProvenance::Unresolved
            } else {
                NoteProvenance::AgentExplanation
            },
            citation_anchor_ids: if heading == "Metadata" {
                Vec::new()
            } else {
                all_anchor_ids.clone()
            },
        })
        .collect();

    Ok(NoteRevision {
        note_revision_id: format!("note-{}", Uuid::new_v4()),
        course_id: course_id.to_owned(),
        thread_id: thread_id.to_owned(),
        source_id: source.source_id.clone(),
        revision: revision.max(1),
        title: source.display_name.clone(),
        user_note_spec: user_note_spec
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        sections,
        source_map,
        created_at: now.to_owned(),
    })
}

pub fn build_course_memory(
    course: &Course,
    syllabus: Option<&SyllabusRevision>,
    concepts: Vec<CourseConcept>,
    assignments: Vec<CourseAssignment>,
    review_items: Vec<ReviewItem>,
) -> CourseMemory {
    CourseMemory {
        course_id: course.course_id.clone(),
        objectives: syllabus
            .map(|value| value.structure.learning_objectives.clone())
            .unwrap_or_default(),
        concepts: concepts
            .into_iter()
            .filter(|value| value.course_id == course.course_id)
            .collect(),
        assignments: assignments
            .into_iter()
            .filter(|value| value.course_id == course.course_id)
            .collect(),
        review_items: review_items
            .into_iter()
            .filter(|value| value.course_id == course.course_id)
            .collect(),
        user_corrections: Vec::new(),
        exam_scope: Vec::new(),
        mastery_evidence: Vec::new(),
    }
}

pub fn schedule_review(
    mut item: ReviewItem,
    successful: bool,
    confidence: u8,
    now: &str,
) -> Result<ReviewItem, UltraNoteError> {
    let now_timestamp =
        Timestamp::from_str(now).map_err(|_| UltraNoteError::InvalidTimestamp(now.to_owned()))?;
    item.confidence = confidence.min(5);
    if successful {
        item.successful_reviews += 1;
        let base = [1_u32, 3, 7, 14, 30, 60];
        let index = (item.successful_reviews.saturating_sub(1) as usize).min(base.len() - 1);
        let confidence_bonus = u32::from(item.confidence.saturating_sub(3));
        item.interval_days = base[index].saturating_add(confidence_bonus);
    } else {
        item.lapses += 1;
        item.successful_reviews = 0;
        item.interval_days = 1;
    }
    item.next_review_at = now_timestamp
        .checked_add(SignedDuration::from_hours(
            i64::from(item.interval_days) * 24,
        ))
        .map_err(|_| UltraNoteError::InvalidTimestamp(now.to_owned()))?
        .to_string();
    Ok(item)
}

pub fn evaluate_homework_policy(
    course_id: &str,
    kind: HomeworkKind,
    ai_policy: &[String],
    integrity_policy: &[String],
) -> HomeworkPolicyDecision {
    let policy_text = ai_policy
        .iter()
        .chain(integrity_policy)
        .map(|value| value.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let explicit_prohibition = contains_any(
        &policy_text,
        &["no ai", "prohibited", "not permitted", "must not use"],
    );
    let clarification_required = kind == HomeworkKind::Unknown
        || ((kind == HomeworkKind::Graded || kind == HomeworkKind::Exam)
            && policy_text.trim().is_empty());
    let allowed_modes = match kind {
        HomeworkKind::Practice | HomeworkKind::Ungraded if !explicit_prohibition => vec![
            HomeworkAssistanceMode::Explain,
            HomeworkAssistanceMode::Hint,
            HomeworkAssistanceMode::Socratic,
            HomeworkAssistanceMode::ReviewAttempt,
            HomeworkAssistanceMode::WorkedExample,
            HomeworkAssistanceMode::Debug,
        ],
        _ => vec![
            HomeworkAssistanceMode::Explain,
            HomeworkAssistanceMode::Hint,
            HomeworkAssistanceMode::Socratic,
            HomeworkAssistanceMode::ReviewAttempt,
            HomeworkAssistanceMode::Debug,
        ],
    };
    HomeworkPolicyDecision {
        course_id: course_id.to_owned(),
        kind,
        clarification_required,
        allowed_modes,
        direct_submittable_answer_allowed: false,
        rationale: vec![
            "Assistance is for learning and reviewing the student's own work.".to_owned(),
            if clarification_required {
                "The assignment type or course policy must be clarified first.".to_owned()
            } else {
                "The selected modes respect the recorded course policy and task classification."
                    .to_owned()
            },
        ],
        prohibited_actions: vec![
            "Submit or impersonate the student".to_owned(),
            "Invent data, citations, results, or source support".to_owned(),
            "Bypass an exam or assessment restriction".to_owned(),
            "Provide a directly submittable answer when course policy prohibits it".to_owned(),
        ],
    }
}

pub fn export_course_markdown(workspace: &UltraNoteWorkspace) -> String {
    let mut output = String::new();
    if let Some(course) = &workspace.course {
        output.push_str(&format!("# {}\n\n", course.title));
        if let Some(code) = &course.code {
            output.push_str(&format!("**{}**\n\n", code));
        }
    }
    if let Some(syllabus) = &workspace.syllabus {
        if !syllabus.structure.learning_objectives.is_empty() {
            output.push_str("## Learning objectives\n\n");
            for objective in &syllabus.structure.learning_objectives {
                output.push_str(&format!("- {objective}\n"));
            }
            output.push('\n');
        }
        let concepts = syllabus
            .structure
            .topic_schedule
            .iter()
            .chain(syllabus.structure.learning_objectives.iter())
            .filter(|value| !value.trim().is_empty())
            .take(12)
            .collect::<Vec<_>>();
        if concepts.len() >= 4 {
            output.push_str("## Concept map\n\n```mermaid\nmindmap\n  root((Course))\n");
            for concept in concepts {
                output.push_str(&format!("    {}\n", mermaid_label(concept)));
            }
            output.push_str("```\n\n");
        }
    }
    for note in &workspace.latest_notes {
        output.push_str(&format!("## {}\n\n", note.title));
        for section in &note.sections {
            let items = section
                .content
                .iter()
                .filter(|item| is_note_content(item))
                .collect::<Vec<_>>();
            if section.heading == "Metadata" || items.is_empty() {
                continue;
            }
            output.push_str(&format!("### {}\n\n", section.heading));
            for item in items {
                output.push_str(&format!("- {item}\n"));
            }
            output.push('\n');
        }
        if !note.source_map.is_empty() {
            output.push_str("### Sources\n\n");
            for anchor in &note.source_map {
                output.push_str(&format!("- {}\n", anchor.excerpt));
            }
            output.push('\n');
        }
    }
    output
}

fn is_note_content(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    ![
        "source:",
        "source kind:",
        "imported:",
        "confirm ",
        "list prerequisite",
        "add worked examples",
        "resolve ambiguous",
        "close the notes",
        "create flashcards only",
        "classify each task",
        "no derivation is attributed",
        "no instructor emphasis",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn mermaid_label(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '(' | ')' | '[' | ']' | '{' | '}' | '"'))
        .take(72)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn validate_input(
    course_id: &str,
    thread_id: Option<&str>,
    content: &str,
) -> Result<(), UltraNoteError> {
    if course_id.trim().is_empty() {
        return Err(UltraNoteError::MissingCourseId);
    }
    if thread_id.is_some_and(|value| value.trim().is_empty()) {
        return Err(UltraNoteError::MissingThreadId);
    }
    if content.trim().is_empty() {
        return Err(UltraNoteError::EmptySource);
    }
    Ok(())
}

fn meaningful_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn push_missing(ambiguities: &mut Vec<String>, values: &[String], message: &str) {
    if values.is_empty() {
        ambiguities.push(message.to_owned());
    }
}

fn fallback(mut values: Vec<String>, message: &str) -> Vec<String> {
    if values.is_empty() {
        values.push(message.to_owned());
    }
    values
}

fn deduplicate_structure(structure: &mut SyllabusStructure) {
    for values in [
        &mut structure.course_information,
        &mut structure.learning_objectives,
        &mut structure.meeting_times,
        &mut structure.topic_schedule,
        &mut structure.assessments,
        &mut structure.grading,
        &mut structure.textbooks,
        &mut structure.late_policy,
        &mut structure.attendance_policy,
        &mut structure.ai_policy,
        &mut structure.academic_integrity_policy,
    ] {
        values.sort();
        values.dedup();
    }
}

fn build_anchors(course_id: &str, source: &CourseSource, lines: &[String]) -> Vec<CitationAnchor> {
    lines
        .chunks(4)
        .enumerate()
        .map(|(index, chunk)| {
            let start = index * 4 + 1;
            let end = start + chunk.len().saturating_sub(1);
            CitationAnchor {
                anchor_id: format!("anchor-{}", Uuid::new_v4()),
                course_id: course_id.to_owned(),
                source_id: source.source_id.clone(),
                page: None,
                slide: None,
                timestamp: None,
                text_range: Some(format!("lines {start}-{end}")),
                excerpt: chunk.join(" ").chars().take(240).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use lunascope_core::{CourseSourceKind, UltraNoteMode};

    use super::*;

    fn course() -> Course {
        Course {
            course_id: "course-1".to_owned(),
            title: "Systems".to_owned(),
            code: Some("CS-501".to_owned()),
            mode: UltraNoteMode::Full,
            current_syllabus_revision: None,
            created_at: "2026-07-27T00:00:00Z".to_owned(),
            updated_at: "2026-07-27T00:00:00Z".to_owned(),
        }
    }

    fn source(kind: CourseSourceKind) -> CourseSource {
        CourseSource {
            source_id: "source-1".to_owned(),
            course_id: "course-1".to_owned(),
            kind,
            display_name: "Week 1".to_owned(),
            content_sha256: "abc".to_owned(),
            imported_at: "2026-07-27T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn syllabus_never_guesses_missing_exam_or_ai_policy() {
        let result = parse_syllabus(
            &course(),
            &source(CourseSourceKind::Syllabus),
            "Course: Systems\nObjective: explain scheduling\nWeek 1: processes",
            1,
            "2026-07-27T00:00:00Z",
        )
        .unwrap();

        assert!(result.structure.assessments.is_empty());
        assert!(result.structure.ai_policy.is_empty());
        assert!(result.ambiguities.iter().any(|item| item.contains("exam")));
        assert!(
            result
                .ambiguities
                .iter()
                .any(|item| item.contains("AI-use"))
        );
        assert_eq!(result.prestudy_plan.len(), 3);
    }

    #[test]
    fn lecture_notes_use_uniform_sections_and_source_anchors() {
        let result = generate_lecture_notes(
            "course-1",
            "thread-1",
            &source(CourseSourceKind::LectureNotes),
            "A process is a program in execution.\nScheduling selects the next runnable process.",
            Some("Prefer concise bullet points."),
            1,
            "2026-07-27T00:00:00Z",
        )
        .unwrap();

        assert_eq!(result.sections.len(), NOTE_HEADINGS.len());
        assert_eq!(result.sections[0].heading, "Metadata");
        assert_eq!(
            result.user_note_spec.as_deref(),
            Some("Prefer concise bullet points.")
        );
        assert!(!result.source_map.is_empty());
        assert!(
            result
                .sections
                .iter()
                .any(|section| section.provenance == NoteProvenance::Unresolved)
        );
    }

    #[test]
    fn review_schedule_is_simple_and_reproducible() {
        let item = ReviewItem {
            review_item_id: "review-1".to_owned(),
            course_id: "course-1".to_owned(),
            concept_id: "concept-1".to_owned(),
            prompt: "What is a process?".to_owned(),
            answer: "A program in execution.".to_owned(),
            interval_days: 0,
            next_review_at: "2026-07-27T00:00:00Z".to_owned(),
            confidence: 0,
            successful_reviews: 0,
            lapses: 0,
            source_anchor_ids: vec![],
        };
        let first = schedule_review(item, true, 3, "2026-07-27T00:00:00Z").unwrap();
        assert_eq!(first.interval_days, 1);
        let second = schedule_review(first, true, 3, "2026-07-28T00:00:00Z").unwrap();
        assert_eq!(second.interval_days, 3);
        let lapsed = schedule_review(second, false, 1, "2026-07-29T00:00:00Z").unwrap();
        assert_eq!(lapsed.interval_days, 1);
        assert_eq!(lapsed.lapses, 1);
    }

    #[test]
    fn unknown_or_graded_work_never_gets_direct_submission_mode() {
        let unknown = evaluate_homework_policy("course-1", HomeworkKind::Unknown, &[], &[]);
        assert!(unknown.clarification_required);
        assert!(!unknown.direct_submittable_answer_allowed);

        let graded = evaluate_homework_policy(
            "course-1",
            HomeworkKind::Graded,
            &["AI use is prohibited".to_owned()],
            &[],
        );
        assert!(
            !graded
                .allowed_modes
                .contains(&HomeworkAssistanceMode::WorkedExample)
        );
        assert!(!graded.direct_submittable_answer_allowed);
    }

    #[test]
    fn course_export_is_note_only_and_adds_a_useful_mermaid_map() {
        let mut syllabus = parse_syllabus(
            &course(),
            &source(CourseSourceKind::Syllabus),
            "Objective: explain scheduling\nWeek 1: processes\nWeek 2: threads\nWeek 3: scheduling\nWeek 4: synchronization",
            1,
            "2026-07-27T00:00:00Z",
        )
        .unwrap();
        syllabus.structure.topic_schedule = vec![
            "Processes".to_owned(),
            "Threads".to_owned(),
            "Scheduling".to_owned(),
            "Synchronization".to_owned(),
        ];
        let note = generate_lecture_notes(
            "course-1",
            "thread-1",
            &source(CourseSourceKind::LectureNotes),
            "A process is a program in execution.\nScheduling selects the next runnable process.",
            None,
            1,
            "2026-07-27T00:00:00Z",
        )
        .unwrap();
        let markdown = export_course_markdown(&UltraNoteWorkspace {
            thread_id: "thread-1".to_owned(),
            binding: None,
            course: Some(course()),
            syllabus: Some(syllabus),
            latest_notes: vec![note],
            memory: None,
            requested_action: "export".to_owned(),
        });

        assert!(markdown.starts_with("# Systems"));
        assert!(markdown.contains("```mermaid\nmindmap"));
        assert!(!markdown.contains("UltraNote course export"));
        assert!(!markdown.contains("Course ID"));
        assert!(!markdown.contains("Provenance"));
        assert!(!markdown.contains("Close the notes"));
    }
}
