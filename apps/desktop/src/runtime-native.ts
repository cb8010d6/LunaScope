import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AgentPlan,
  ConversationMessage,
  Course,
  CourseSearchHit,
  CourseSourceKind,
  DomainPackDescriptor,
  DomainPackId,
  GithubImportPreview,
  HomeworkKind,
  HomeworkPolicyDecision,
  ImportComparison,
  ImportedAttachment,
  InstalledImportVersion,
  LoadedSkill,
  McpInvocationResult,
  McpServerConfig,
  McpServerStatus,
  ModelAssignment,
  ModelRole,
  ModelRoutingPolicy,
  ModelReplyLanguage,
  ModelSelectionSettings,
  NaturalLanguageRoutingDraft,
  OrchestrationPatch,
  OrchestrationChangeSet,
  OrchestrationRunResult,
  OrchestrationSession,
  ProviderConfig,
  ProviderHeader,
  ProviderProtocol,
  ProviderType,
  ProjectKind,
  ReasoningSummarySource,
  LunaProject,
  RuntimeDelta,
  RuntimeSnapshot,
  SkillSummary,
  ToolRoutingDecision,
  ToolRoutingRequest,
  UiLanguage,
  UltraNoteWorkspace,
  UserPreferences,
  WorkerSpec,
} from "../../../packages/runtime-contract/src/types.generated";

type RuntimeMessage =
  | { kind: "snapshot"; data: RuntimeSnapshot }
  | { kind: "delta"; data: RuntimeDelta };

type OrchestrationProgress = {
  workerId: string | null;
  role: string;
  state: string;
  detail: string;
  tool: string | null;
  plan: OrchestrationSession["plan"] | null;
  itemId: string | null;
  itemPhase: string | null;
  summarySource: ReasoningSummarySource | null;
  summaryIndex: number | null;
  agentPlan: AgentPlan | null;
};

type RetryOrchestrationOutcome = {
  runId: string;
  result: OrchestrationRunResult;
  retriedWorkerIds: string[];
  resumedWorkerIds: string[];
};

type NativeOrchestrationOutcome = {
  session: OrchestrationSession;
  result: OrchestrationRunResult;
  repairCycles: number;
  sourceRunIds: string[];
};

type RecoveredOrchestrationView = {
  session: OrchestrationSession;
  result: OrchestrationRunResult;
  sourceRunIds: string[];
};

type GuidanceReplanOutcome = {
  runId: string;
  guidance: string;
  affectedWorkerIds: string[];
  deferredWorkerIds: string[];
};

type ExtensionDirectories = {
  root: string;
  systemSkills: string;
  userSkills: string;
  tools: string;
  mcp: string;
};

type ActiveConversationReference = {
  threadId: string;
  runId: string | null;
  projectId: string | null;
};

type LunaScopeUiBridge = {
  newConversation(thread: {
    id: string;
    title: string;
    projectId: string;
    runId: string;
  }): void;
  renameConversation(threadId: string, title: string): boolean;
  bindConversationRun(threadId: string, runId: string): boolean;
  activateConversation(threadId: string): ActiveConversationReference | null;
  activeConversation(): ActiveConversationReference | null;
  removeConversation(threadId: string): {
    deleted: boolean;
    wasActive: boolean;
    runId: string | null;
    projectId: string | null;
    activeConversation: ActiveConversationReference | null;
  };
  removeProjectConversations(projectId: string): {
    removed: number;
    removedThreadIds: string[];
    removedActive: boolean;
    activeConversation: ActiveConversationReference | null;
  };
  clearOrchestration(): void;
  setRuntimeProjection(projection: {
    plan: OrchestrationSession["plan"] | null;
    result: OrchestrationRunResult | null;
    changeSets: OrchestrationChangeSet[];
    agentPlans: Record<string, AgentPlan>;
  }): void;
  setExecutionActivity(activity: {
    running: boolean;
    workerId: string | null;
    role: string;
    state: string;
    detail: string;
  }): void;
  mergeConversationEvents(
    threadId: string,
    events: Array<{
      id: string;
      type: string;
      title: string;
      summary: string;
      time: string;
      createdAt: string;
      sequence: number;
    }>,
  ): void;
  appendConversationEvent(event: {
    id?: string;
    type: string;
    title: string;
    summary: string;
    openOrchestration?: boolean;
    retryConversation?: boolean;
    trackHistory?: boolean;
  }): void;
};

declare global {
  interface Window {
    lunaScopeUi?: LunaScopeUiBridge;
  }
}

const bootstrapRunId = "run-native-bootstrap";
const globalRoutingScope = "global";
const retrySourceRunStorageKey = "lunascope.native.retrySourceRunId";
const automaticFailedWorkerRetryLimit = 2;
const workerRoleCatalog = [
  ["planner", "Planner / 规划", "planning, coordination"],
  ["builder", "Builder / 通用实现", "implementation"],
  ["frontend", "Frontend / 前端", "frontend, ui"],
  ["backend", "Backend / 后端", "backend, runtime"],
  ["researcher", "Researcher / 科研", "research, evidence"],
  ["reviewer", "Reviewer / 审阅", "review, read-only"],
  ["verifier", "Verifier / 验证", "verification, independent"],
  ["game_designer", "Game Designer / 游戏", "game-development, design"],
  ["academic_writer", "Academic Writer / 学术写作", "academic, writing"],
  ["documentation", "Documentation / 文档", "documentation, writing"],
] as const;

let providerConfigs: ProviderConfig[] = [];
let domainPacks: DomainPackDescriptor[] = [];
let skillSummaries: SkillSummary[] = [];
let mcpServerConfigs: McpServerConfig[] = [];
let githubImportPreview: GithubImportPreview | null = null;
let installedImportVersions: InstalledImportVersion[] = [];
let nativeOrchestrationSession: OrchestrationSession | null = null;
let nativeOrchestrationResult: OrchestrationRunResult | null = null;
let nativeChangeSets: OrchestrationChangeSet[] = [];
let nativeSourceRunIds: string[] = [];
let nativeProjectionPlan: OrchestrationSession["plan"] | null = null;
let nativeAgentPlans: Record<string, AgentPlan> = {};
let nativeRetrySourceRunId: string | null = localStorage.getItem(
  retrySourceRunStorageKey,
);
let nativeOrchestrationRunning = false;
let nativeOrchestrationPaused = false;
const runtimeReasoningText = new Map<string, string>();
let nativeInspector:
  | { kind: "orchestrator" | "worker" | "synthesis"; workerIndex?: number }
  | null = null;
let nativeInspectorTab = "overview";
let nativeOrchestrationMode:
  | "blueprint"
  | "runtime"
  | "queue"
  | "handoffs" = "blueprint";
let ultraNoteWorkspace: UltraNoteWorkspace | null = null;
let ultraNoteCourses: Course[] = [];
let ultraNotePolicy: HomeworkPolicyDecision | null = null;
let ultraNoteSearchHits: CourseSearchHit[] = [];
let projects: LunaProject[] = [];
let activeProjectId: string | null = null;
let activeConversationRunId: string | null = null;
let activeConversationThreadId: string | null = null;
const orchestrationByThread = new Map<
  string,
  {
    session: OrchestrationSession | null;
    plan: OrchestrationSession["plan"] | null;
    result: OrchestrationRunResult | null;
    retrySourceRunId: string | null;
    changeSets: OrchestrationChangeSet[];
    sourceRunIds: string[];
    agentPlans: Record<string, AgentPlan>;
  }
>();
let orchestrationHydrationGeneration = 0;
let projectDraftFolders: string[] = [];
let projectWorkspacePath = "";
let editingProjectId: string | null = null;
let projectDraftName = "";
let projectDraftKind: ProjectKind = "general";
let projectDraftCourseTitle = "";
let projectDraftCourseCode = "";
let projectDraftSyllabus: ImportedAttachment | null = null;
let userPreferences: UserPreferences = {
  language: "chinese",
  modelReplyLanguage: "follow_ui",
  ultranoteNoteSpec:
    "Use clear hierarchical headings, preserve source anchors, explain concepts in plain language, and separate source content from LunaScope explanations. When source and output languages differ, show difficult or technical terms with their English form in parentheses and finish with a glossary. Preserve formulas as defined, checkable derivations and explain every chart or function graph.",
};
const mcpServerStatuses = new Map<string, McpServerStatus>();
let activeWorkspaceRoot = "";
let pendingAttachments: ImportedAttachment[] = [];
type ComputerAccessMode =
  | "request_approval"
  | "self_approve"
  | "full_access";
const accessSettingsKey = "lunascope-native-access-settings";
let computerAccessMode: ComputerAccessMode = "self_approve";
let bypassMode = false;
let extensionDirectories: ExtensionDirectories = {
  root: "D:\\LunaScopeData\\extensions",
  systemSkills: "D:\\LunaScopeData\\extensions\\skills\\system",
  userSkills: "D:\\LunaScopeData\\extensions\\skills\\user",
  tools: "D:\\LunaScopeData\\extensions\\tools",
  mcp: "D:\\LunaScopeData\\extensions\\mcp",
};
let routingPolicy: ModelRoutingPolicy = {
  priorities: { quality: 80, cost: 50, speed: 50, privacy: 50 },
  allowedProviderConfigIds: [],
  disallowedProviderConfigIds: [],
  maximumCostMicrousdPerRun: null,
  preferLocal: false,
  requiredCapabilities: ["text"],
  fallbackAllowed: true,
  askBeforeCostEscalation: true,
  rolePreferences: [],
};
let pendingRoutingDraft: NaturalLanguageRoutingDraft | null = null;
let modelSelectionSettings: ModelSelectionSettings = {
  orchestration: {
    role: "orchestration",
    providerConfigId: "openai-primary",
    modelId: "gpt-5-mini",
    reasoningEffort: "high",
    maximumContextTokens: null,
    maximumBudgetMicrousd: null,
    fallbackProviderConfigId: null,
    fallbackModelId: null,
    locked: false,
  },
  workerPool: [
    ["programming", "openai-primary", "gpt-5-mini"],
    ["writing", "anthropic-primary", "claude-sonnet"],
    ["fast_cheap", "deepseek-primary", "deepseek-v4-flash"],
  ].map(([role, providerConfigId, modelId]) => ({
    role: role as ModelRole,
    providerConfigId,
    modelId,
    reasoningEffort: "medium",
    maximumContextTokens: null,
    maximumBudgetMicrousd: null,
    fallbackProviderConfigId: null,
    fallbackModelId: null,
    locked: false,
  })),
};

function escapeHtml(value: unknown): string {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function loadAccessSettings(): void {
  try {
    const stored = JSON.parse(
      localStorage.getItem(accessSettingsKey) ?? "{}",
    ) as { mode?: ComputerAccessMode; bypass?: boolean };
    if (
      stored.mode === "request_approval" ||
      stored.mode === "self_approve" ||
      stored.mode === "full_access"
    ) {
      computerAccessMode = stored.mode;
    }
    bypassMode = Boolean(stored.bypass);
  } catch {
    computerAccessMode = "self_approve";
    bypassMode = false;
  }
}

function applyAccessSettings(): void {
  const mode =
    document.querySelector<HTMLSelectElement>("#computerAccessMode");
  const bypass = document.querySelector<HTMLInputElement>("#bypassMode");
  if (mode) mode.value = computerAccessMode;
  if (bypass) bypass.checked = bypassMode;
  document.documentElement.dataset.computerAccess = computerAccessMode;
  document.documentElement.dataset.bypassMode = String(bypassMode);
}

function saveAccessSettings(): void {
  localStorage.setItem(
    accessSettingsKey,
    JSON.stringify({ mode: computerAccessMode, bypass: bypassMode }),
  );
  applyAccessSettings();
}

function tr(chinese: string, english: string): string {
  return userPreferences.language === "chinese" ? chinese : english;
}

function applyUiLanguage(): void {
  const chinese = userPreferences.language === "chinese";
  document.documentElement.lang = chinese ? "zh-CN" : "en";
  document.documentElement.dataset.uiLanguage = userPreferences.language;
  const text = (
    selector: string,
    chineseText: string,
    englishText: string,
  ): void => {
    const element = document.querySelector<HTMLElement>(selector);
    if (element) element.textContent = chinese ? chineseText : englishText;
  };
  text(
    "#newTask span",
    activeProjectId ? "新建对话" : "新建项目",
    activeProjectId ? "New conversation" : "New project",
  );
  text(
    '[data-special="orchestration"] span:last-child',
    "多 Agent 编排",
    "Orchestration",
  );
  text('[data-special="memory"] span:last-child', "记忆", "Memory");
  text(
    '[data-special="integrations"] span:last-child',
    "技能与 MCP",
    "Skills & MCP",
  );
  text('[data-special="settings"] span:last-child', "设置", "Settings");
  text("#send", "发送", "Send");
  text("#headerMeta", "本地 Agent 工作台", "Local agent workspace");
  text("#runConfig", "选择模型 · 自动路由", "Select model · Auto routing");
  document
    .querySelectorAll<HTMLElement>("[data-empty-title-zh]")
    .forEach((card) => {
      const title = card.querySelector<HTMLElement>("strong");
      const help = card.querySelector<HTMLElement>(".settings-help");
      if (title) {
        title.textContent = chinese
          ? (card.dataset.emptyTitleZh ?? "")
          : (card.dataset.emptyTitleEn ?? "");
      }
      if (help) {
        help.textContent = chinese
          ? (card.dataset.emptyHelpZh ?? "")
          : (card.dataset.emptyHelpEn ?? "");
      }
    });
  const settingsLabels: Record<string, [string, string]> = {
    general: ["通用", "General"],
    providers: ["模型与提供商", "Models & Providers"],
    "skills-mcp": ["技能与 MCP", "Skills & MCP"],
    projects: ["项目与文件夹", "Projects & Folders"],
    ultranote: ["UltraNote", "UltraNote"],
  };
  document
    .querySelectorAll<HTMLButtonElement>("[data-settings-page]")
    .forEach((button) => {
      const labels = settingsLabels[button.dataset.settingsPage ?? ""];
      if (labels) button.textContent = chinese ? labels[0] : labels[1];
    });
  const composer =
    document.querySelector<HTMLTextAreaElement>("#composerInput");
  if (composer) {
    composer.placeholder = chinese
      ? "告诉 LunaScope 下一步要做什么，输入 / 查看功能…"
      : "Tell LunaScope what to do next, or type / for commands…";
  }
  const project = projects.find((item) => item.projectId === activeProjectId);
  const switcher = document.querySelector<HTMLElement>("#projectSwitch");
  if (switcher) {
    switcher.innerHTML = project
      ? `<strong>${escapeHtml(project.name)}</strong><span>${escapeHtml(project.folders.find((folder) => folder.isWorkspace)?.path ?? "")}</span>`
      : `<strong>${tr("未选择项目", "No project selected")}</strong><span>${tr("新建或选择项目", "Create or select a project")}</span>`;
  }
}

async function loadUserPreferences(): Promise<void> {
  if (isTauri()) {
    userPreferences = await invoke<UserPreferences>("get_user_preferences");
  }
  applyUiLanguage();
}

function ensureProjectDialog(): HTMLDialogElement {
  let dialog = document.querySelector<HTMLDialogElement>(
    "#nativeProjectDialog",
  );
  if (dialog) return dialog;
  dialog = document.createElement("dialog");
  dialog.id = "nativeProjectDialog";
  document.body.append(dialog);
  return dialog;
}

function projectFolderRows(): string {
  if (!projectDraftFolders.length) {
    return `<p class="ultranote-empty">${tr("尚未选择文件夹。一个项目可包含多个文件夹。", "No folders selected. A project can contain multiple folders.")}</p>`;
  }
  return projectDraftFolders
    .map(
      (path, index) => `
      <div class="project-folder-row">
        <label><input type="radio" name="projectWorkspaceFolder" value="${escapeHtml(path)}" ${path === projectWorkspacePath ? "checked" : ""}> ${tr("Workspace", "Workspace")}</label>
        <span title="${escapeHtml(path)}">${escapeHtml(path)}</span>
        <button class="text-btn" type="button" data-native-action="project-remove-folder" data-folder-index="${index}">${tr("移除", "Remove")}</button>
      </div>`,
    )
    .join("");
}

function captureProjectDraftFields(): void {
  projectDraftName =
    document.querySelector<HTMLInputElement>("#nativeProjectName")?.value ??
    projectDraftName;
  projectDraftKind =
    document.querySelector<HTMLSelectElement>("#nativeProjectKind")?.value as
      | ProjectKind
      | undefined ?? projectDraftKind;
  projectDraftCourseTitle =
    document.querySelector<HTMLInputElement>("#nativeCourseTitle")?.value ??
    projectDraftCourseTitle;
  projectDraftCourseCode =
    document.querySelector<HTMLInputElement>("#nativeCourseCode")?.value ??
    projectDraftCourseCode;
}

function renderProjectDialog(): void {
  const dialog = ensureProjectDialog();
  const selected = projects.find(
    (project) => project.projectId === activeProjectId,
  );
  dialog.innerHTML = `
    <div class="dialog wide">
      <div class="dialog-head"><div><strong>${tr("项目管理", "Project management")}</strong><div class="mono">${tr("多文件夹 · 单一 Workspace", "Multiple folders · one workspace")}</div></div><button class="icon-btn" type="button" data-native-action="project-close">×</button></div>
      <div class="dialog-body project-manager">
        <aside class="project-list">
          <div class="pane-head" style="padding-inline:0"><strong>${tr("项目", "Projects")}</strong><button class="ghost-btn" type="button" data-native-action="project-new">${tr("新建", "New")}</button></div>
          ${
            projects.length
              ? projects
                  .map(
                    (project) => `<button class="list-item ${project.projectId === activeProjectId ? "active" : ""}" type="button" data-native-action="project-edit" data-project-id="${escapeHtml(project.projectId)}"><strong>${escapeHtml(project.name)}</strong><span>${project.folders.length} ${tr("个文件夹", "folders")} · ${escapeHtml(project.folders.find((folder) => folder.isWorkspace)?.displayName ?? "")}</span></button>`,
                  )
                  .join("")
              : `<p class="ultranote-empty">${tr("还没有项目。", "No projects yet.")}</p>`
          }
        </aside>
        <section class="project-editor">
          <div class="field"><label for="nativeProjectName">${tr("项目名称", "Project name")}</label><input class="input" id="nativeProjectName" value="${escapeHtml(projectDraftName)}" placeholder="${tr("我的项目", "My project")}"></div>
          <div class="field"><label for="nativeProjectKind">${tr("项目类型", "Project type")}</label><select class="select" id="nativeProjectKind"><option value="general" ${projectDraftKind === "general" ? "selected" : ""}>${tr("通用 Agent 项目", "General Agent project")}</option><option value="ultra_note" ${projectDraftKind === "ultra_note" ? "selected" : ""}>UltraNote</option></select><span class="settings-help">${tr("UltraNote 项目中的每个对话都会自动使用课程上下文。", "Every conversation in an UltraNote project automatically uses its course context.")}</span></div>
          ${projectDraftKind === "ultra_note" ? `<section class="settings-card ultranote-project-onboarding"><div class="settings-card-head"><div><strong>${tr("课程基础信息", "Course foundation")}</strong><span>${tr("课程大纲由编排模型解析，并持久化到项目课程记忆。", "The Orchestration model parses the syllabus into persistent course memory.")}</span></div></div><div class="settings-grid"><div class="field"><label for="nativeCourseTitle">${tr("课程名称", "Course name")}</label><input class="input" id="nativeCourseTitle" value="${escapeHtml(projectDraftCourseTitle)}"></div><div class="field"><label for="nativeCourseCode">${tr("课程编号", "Course code")}</label><input class="input" id="nativeCourseCode" value="${escapeHtml(projectDraftCourseCode)}"></div></div><div class="field"><label>${tr("课程大纲", "Syllabus")}</label><div class="row"><button class="ghost-btn" type="button" data-native-action="project-select-syllabus">${tr("选择课程大纲", "Choose syllabus")}</button>${projectDraftSyllabus ? `<span class="attachment-chip">${escapeHtml(projectDraftSyllabus.displayName)}</span>` : `<span class="settings-help">${tr("支持 PDF、PPTX、DOCX、XLSX、Markdown 和文本。", "PDF, PPTX, DOCX, XLSX, Markdown, and text are supported.")}</span>`}</div></div></section>` : ""}
          <div class="pane-head" style="padding-inline:0"><div><strong>${tr("项目文件夹", "Project folders")}</strong><span class="settings-help">${tr("选择一个或多个文件夹，并指定其中一个作为 Agent 的 workspace。", "Choose one or more folders and designate one as the Agent workspace.")}</span></div><button class="ghost-btn" type="button" data-native-action="project-select-folders">${tr("选择文件夹", "Choose folders")}</button></div>
          <div class="project-folder-list">${projectFolderRows()}</div>
          <div class="settings-status" id="projectActionStatus" role="status"></div>
        </section>
      </div>
      <div class="dialog-foot"><span class="settings-help">${selected ? tr(`当前项目：${selected.name}`, `Active project: ${selected.name}`) : tr("未选择当前项目", "No active project")}</span><div class="row">${editingProjectId ? `<button class="text-btn" style="color:var(--danger)" type="button" data-native-action="project-delete" data-project-id="${escapeHtml(editingProjectId)}">${tr("删除项目", "Delete project")}</button>` : ""}<button class="ghost-btn" type="button" data-native-action="project-close">${tr("取消", "Cancel")}</button><button class="primary-btn" type="button" data-native-action="project-save">${tr("保存项目", "Save project")}</button></div></div>
    </div>`;
}

async function openProjectManager(createNew = false): Promise<void> {
  if (isTauri()) {
    projects = await invoke<LunaProject[]>("list_projects");
  }
  if (createNew) {
    editingProjectId = null;
    projectDraftName = "";
    projectDraftKind = "general";
    projectDraftCourseTitle = "";
    projectDraftCourseCode = "";
    projectDraftSyllabus = null;
    projectDraftFolders = [];
    projectWorkspacePath = "";
  } else {
    const selected =
      projects.find((project) => project.projectId === activeProjectId) ??
      projects[0];
    if (selected) {
      activeProjectId = selected.projectId;
      editingProjectId = selected.projectId;
      projectDraftName = selected.name;
      projectDraftKind = selected.kind;
      projectDraftCourseTitle = selected.ultranote?.courseTitle ?? "";
      projectDraftCourseCode = selected.ultranote?.courseCode ?? "";
      projectDraftSyllabus = selected.ultranote
        ? {
            attachmentId: selected.ultranote.syllabusAttachmentId,
            displayName: selected.ultranote.syllabusDisplayName,
            mediaType: "application/octet-stream",
            kind: "document",
            sizeBytes: 0,
            extractedCharacters: 0,
            embeddedImageCount: 0,
            previewPath: null,
            extractionWarning: null,
          }
        : null;
      projectDraftFolders = selected.folders.map((folder) => folder.path);
      projectWorkspacePath =
        selected.folders.find((folder) => folder.isWorkspace)?.path ?? "";
    }
  }
  renderProjectDialog();
  ensureProjectDialog().showModal();
}

function activeProject(): LunaProject | undefined {
  return projects.find((project) => project.projectId === activeProjectId);
}

function syncActiveWorkspace(): string {
  const workspace =
    activeProject()?.folders.find((folder) => folder.isWorkspace)?.path ?? "";
  projectWorkspacePath = workspace;
  activeWorkspaceRoot = workspace;
  return workspace;
}

function persistActiveOrchestrationState(): void {
  if (!activeConversationThreadId) return;
  if (
    !nativeOrchestrationSession &&
    !nativeOrchestrationResult &&
    !nativeRetrySourceRunId
  ) {
    orchestrationByThread.delete(activeConversationThreadId);
    return;
  }
  orchestrationByThread.set(activeConversationThreadId, {
    session: nativeOrchestrationSession,
    plan: nativeProjectionPlan,
    result: nativeOrchestrationResult,
    retrySourceRunId: nativeRetrySourceRunId,
    changeSets: nativeChangeSets,
    sourceRunIds: nativeSourceRunIds,
    agentPlans: nativeAgentPlans,
  });
}

function applyActiveConversationReference(
  reference: ActiveConversationReference | null,
  discardPrevious = false,
): void {
  if (!discardPrevious) persistActiveOrchestrationState();
  activeConversationThreadId = reference?.threadId ?? null;
  activeConversationRunId = reference?.runId ?? null;
  if (
    reference?.projectId &&
    projects.some((project) => project.projectId === reference.projectId)
  ) {
    activeProjectId = reference.projectId;
  }
  const stored = reference
    ? orchestrationByThread.get(reference.threadId)
    : undefined;
  nativeOrchestrationSession = stored?.session ?? null;
  nativeProjectionPlan = stored?.plan ?? stored?.session?.plan ?? null;
  nativeOrchestrationResult = stored?.result ?? null;
  nativeChangeSets = stored?.changeSets ?? [];
  nativeSourceRunIds = stored?.sourceRunIds ?? [];
  nativeAgentPlans = stored?.agentPlans ?? {};
  nativeRetrySourceRunId = stored?.retrySourceRunId ?? null;
  if (nativeRetrySourceRunId) {
    localStorage.setItem(retrySourceRunStorageKey, nativeRetrySourceRunId);
  } else {
    localStorage.removeItem(retrySourceRunStorageKey);
  }
  if (!nativeOrchestrationSession) window.lunaScopeUi?.clearOrchestration();
  syncActiveWorkspace();
  applyUiLanguage();
  renderNativeOrchestration();
  if (reference?.threadId) {
    void hydrateDurableConversation(reference.threadId);
  }
  if (
    reference?.runId &&
    (!stored?.result || !stored.sourceRunIds?.length)
  ) {
    void hydrateActiveConversationRuntime(reference);
  }
}

async function hydrateDurableConversation(threadId: string): Promise<void> {
  if (!isTauri()) return;
  try {
    const messages = await invoke<ConversationMessage[]>(
      "list_conversation_messages",
      { threadId },
    );
    if (activeConversationThreadId !== threadId) return;
    window.lunaScopeUi?.mergeConversationEvents(
      threadId,
      messages.map((message) => ({
        id: message.messageId,
        type: message.role === "user" ? "user_message" : "assistant_message",
        title: message.role === "user" ? "You" : "LunaScope",
        summary: message.content,
        createdAt: message.createdAt,
        time: new Date(message.createdAt).toLocaleTimeString(
          userPreferences.language === "english" ? "en-US" : "zh-CN",
          { hour: "2-digit", minute: "2-digit" },
        ),
        sequence: message.sequence,
      })),
    );
  } catch (error) {
    console.error("Failed to hydrate durable conversation messages", error);
  }
}

async function hydrateActiveConversationRuntime(
  reference: ActiveConversationReference,
): Promise<void> {
  if (!isTauri() || !reference.runId) return;
  const generation = ++orchestrationHydrationGeneration;
  try {
    const recovered = await invoke<RecoveredOrchestrationView | null>(
      "recover_native_orchestration_view",
      { runId: reference.runId },
    );
    if (
      generation !== orchestrationHydrationGeneration ||
      activeConversationThreadId !== reference.threadId ||
      activeConversationRunId !== reference.runId ||
      !recovered
    ) {
      return;
    }
    nativeOrchestrationSession = recovered.session;
    nativeProjectionPlan = recovered.session.plan;
    nativeOrchestrationResult = recovered.result;
    nativeSourceRunIds = recovered.sourceRunIds;
    nativeAgentPlans = recovered.session.snapshot.agentPlans ?? {};
    await refreshNativeChangeSets(nativeSourceRunIds);
    if (
      generation !== orchestrationHydrationGeneration ||
      activeConversationThreadId !== reference.threadId
    ) {
      return;
    }
    persistActiveOrchestrationState();
    renderNativeOrchestration();
  } catch (error) {
    console.error("Failed to recover durable orchestration view", error);
  }
}

function clearActiveOrchestration(): void {
  if (activeConversationThreadId) {
    orchestrationByThread.delete(activeConversationThreadId);
  }
  nativeOrchestrationSession = null;
  nativeRetrySourceRunId = null;
  nativeSourceRunIds = [];
  nativeProjectionPlan = null;
  nativeOrchestrationResult = null;
  nativeChangeSets = [];
  nativeAgentPlans = {};
  nativeInspector = null;
  localStorage.removeItem(retrySourceRunStorageKey);
  window.lunaScopeUi?.clearOrchestration();
  renderNativeOrchestration();
}

function clearCompletedOrchestrationGraph(): void {
  nativeOrchestrationSession = null;
  nativeInspector = null;
  window.lunaScopeUi?.clearOrchestration();
  persistActiveOrchestrationState();
  renderNativeOrchestration();
}

async function deleteConversationById(
  threadId: string,
  title: string,
): Promise<void> {
  if (
    nativeOrchestrationRunning &&
    threadId === activeConversationThreadId
  ) {
    window.alert(
      tr(
        "运行中的对话暂不能删除。请先停止运行。",
        "A running conversation cannot be deleted. Stop the run first.",
      ),
    );
    return;
  }
  if (
    !window.confirm(
      tr(
        `删除“${title}”？此操作无法撤销。`,
        `Delete “${title}”? This cannot be undone.`,
      ),
    )
  ) {
    return;
  }
  if (isTauri()) {
    await invoke<number>("delete_conversation", { threadId });
  }
  const removal = window.lunaScopeUi?.removeConversation(threadId);
  orchestrationByThread.delete(threadId);
  if (removal?.wasActive) {
    applyActiveConversationReference(removal.activeConversation, true);
  }
}

async function createNewConversation(): Promise<void> {
  const project = activeProject();
  if (!project) {
    await openProjectManager(true);
    return;
  }
  const threadId = `thread-${crypto.randomUUID()}`;
  const runId = `run-${crypto.randomUUID()}`;
  syncActiveWorkspace();
  window.lunaScopeUi?.newConversation({
    id: threadId,
    title: tr("等待首条消息", "Awaiting first message"),
    projectId: project.projectId,
    runId,
  });
  applyActiveConversationReference({
    threadId,
    runId,
    projectId: project.projectId,
  });
  if (isTauri()) {
    await invoke<RuntimeSnapshot>("create_run", {
      request: {
        runId,
        title: tr("等待首条消息", "Awaiting first message"),
        initialPrompt: "",
        projectId: project.projectId,
        threadId,
      },
    });
    if (project.kind === "ultra_note" && project.ultranote) {
      await invoke<UltraNoteWorkspace>("bind_course_thread", {
        request: {
          courseId: project.ultranote.courseId,
          threadId,
        },
      });
    }
  }
  document.querySelector<HTMLTextAreaElement>("#composerInput")?.focus();
}

function renderPendingAttachments(): void {
  const box = document.querySelector<HTMLElement>("#composer .composer-box");
  if (!box) return;
  box.querySelector("#pendingAttachments")?.remove();
  if (!pendingAttachments.length) return;
  const rows = pendingAttachments
    .map((attachment) => {
      const details = [
        `${Math.max(1, Math.round(attachment.sizeBytes / 1024))} KB`,
        attachment.extractedCharacters
          ? tr(
              `${attachment.extractedCharacters.toLocaleString()} 字符`,
              `${attachment.extractedCharacters.toLocaleString()} chars`,
            )
          : tr("视觉输入", "visual input"),
        attachment.embeddedImageCount
          ? tr(
              `${attachment.embeddedImageCount} 张内嵌图片`,
              `${attachment.embeddedImageCount} embedded images`,
            )
          : "",
      ]
        .filter(Boolean)
        .join(" · ");
      return `<div class="attachment-chip" title="${escapeHtml(attachment.extractionWarning ?? attachment.displayName)}">
        <span class="attachment-kind">${escapeHtml(attachment.kind.toUpperCase())}</span>
        <span class="attachment-copy"><strong>${escapeHtml(attachment.displayName)}</strong><small>${escapeHtml(details)}</small></span>
        ${attachment.extractionWarning ? '<span class="attachment-warning" aria-label="Extraction warning">!</span>' : ""}
        <button type="button" class="attachment-remove" data-native-action="remove-attachment" data-attachment-id="${escapeHtml(attachment.attachmentId)}" aria-label="${escapeHtml(tr(`移除 ${attachment.displayName}`, `Remove ${attachment.displayName}`))}">×</button>
      </div>`;
    })
    .join("");
  box.insertAdjacentHTML(
    "afterbegin",
    `<div class="attachment-tray" id="pendingAttachments" aria-label="${escapeHtml(tr("待发送附件", "Pending attachments"))}">${rows}</div>`,
  );
}

function renderNativeRunControls(): void {
  const permissions = document.querySelector<HTMLElement>(
    "#composer .composer-permissions",
  );
  const input = document.querySelector<HTMLTextAreaElement>("#composerInput");
  const send = document.querySelector<HTMLButtonElement>("#send");
  if (!permissions || !input || !send) return;
  let controls = permissions.querySelector<HTMLElement>("#nativeRunControls");
  if (!nativeOrchestrationRunning) {
    controls?.remove();
    send.textContent = tr("发送", "Send");
    input.placeholder = tr(
      "告诉 LunaScope 下一步要做什么，输入 / 查看功能…",
      "Tell LunaScope what to do next, or type / for commands…",
    );
    return;
  }
  if (!controls) {
    controls = document.createElement("span");
    controls.id = "nativeRunControls";
    controls.className = "native-run-controls";
    send.before(controls);
  }
  controls.innerHTML = `<button class="ghost-btn" type="button" data-native-run-control="pause">${escapeHtml(nativeOrchestrationPaused ? tr("继续", "Resume") : tr("暂停", "Pause"))}</button><button class="text-btn" type="button" data-native-run-control="cancel">${escapeHtml(tr("取消", "Cancel"))}</button>`;
  send.textContent = tr("引导", "Guide");
  input.placeholder = tr(
    "输入引导；发送后编排模型会重新规划剩余链路…",
    "Send guidance to replan the remaining execution path…",
  );
}

async function setNativeRunPaused(paused: boolean): Promise<void> {
  const changed = await invoke<boolean>("set_native_orchestration_paused", {
    paused,
  });
  if (!changed && !nativeOrchestrationRunning) return;
  nativeOrchestrationPaused = paused;
  renderNativeRunControls();
  window.lunaScopeUi?.setExecutionActivity({
    running: true,
    workerId: null,
    role: "LunaScope",
    state: paused ? tr("已暂停", "Paused") : tr("运行中", "Running"),
    detail: paused
      ? tr("已暂停新步骤；当前安全边界与上下文均已保留", "New steps are paused; the current safe boundary and context are preserved")
      : tr("已继续执行剩余链路", "Continuing the remaining execution path"),
  });
}

async function importAttachmentPaths(paths: string[]): Promise<void> {
  const unique = [...new Set(paths.filter(Boolean))];
  if (!unique.length) return;
  if (!isTauri()) {
    throw new Error(
      tr("浏览器预览不能读取本地附件。", "Browser preview cannot read local attachments."),
    );
  }
  window.lunaScopeUi?.setExecutionActivity({
    running: true,
    workerId: null,
    role: "LunaScope",
    state: tr("正在读取附件", "Reading attachments"),
    detail: tr(
      `正在本地解析 ${unique.length} 个文件，并提取文档结构与图片…`,
      `Parsing ${unique.length} local files and extracting document structure and images…`,
    ),
  });
  try {
    const imported = await invoke<ImportedAttachment[]>("import_attachments", {
      request: { paths: unique },
    });
    const known = new Set(pendingAttachments.map((item) => item.attachmentId));
    pendingAttachments.push(
      ...imported.filter((item) => !known.has(item.attachmentId)),
    );
    renderPendingAttachments();
  } finally {
    window.lunaScopeUi?.setExecutionActivity({
      running: false,
      workerId: null,
      role: "LunaScope",
      state: tr("附件已就绪", "Attachments ready"),
      detail: tr(
        "文件内容已在本地完成安全提取；发送后才会进入所选模型上下文。",
        "Files were safely extracted locally and enter model context only after Send.",
      ),
    });
  }
}

async function chooseAttachments(): Promise<void> {
  const selection = await open({
    multiple: true,
    directory: false,
    title: tr("添加资料或图片", "Add sources or images"),
    filters: [
      {
        name: "Documents and images",
        extensions: [
          "pdf",
          "doc",
          "docx",
          "xls",
          "xlsx",
          "ppt",
          "pptx",
          "png",
          "jpg",
          "jpeg",
          "webp",
          "gif",
          "bmp",
          "md",
          "txt",
          "csv",
          "tsv",
          "json",
          "xml",
          "html",
          "tex",
          "bib",
        ],
      },
    ],
  });
  const paths = !selection
    ? []
    : Array.isArray(selection)
      ? selection
      : [selection];
  await importAttachmentPaths(paths);
}

async function submitConversation(
  objective: string,
  existingMessageId?: string,
): Promise<void> {
  const text = objective.trim();
  if (!text) return;
  if (!activeProject()) {
    await openProjectManager(true);
    throw new Error(tr("请先创建或选择项目。", "Create or select a project first."));
  }
  if (nativeOrchestrationRunning) {
    await submitLiveGuidance(text);
    return;
  }
  if (!activeConversationRunId || !activeConversationThreadId) {
    await createNewConversation();
  }
  const messageId = existingMessageId ?? `message-${crypto.randomUUID()}`;
  const attachmentsForRun = [...pendingAttachments];
  if (!existingMessageId) {
    window.lunaScopeUi?.appendConversationEvent({
      id: messageId,
      type: "user_message",
      title: "You",
      summary:
        attachmentsForRun.length > 0
          ? `${text}\n\n${attachmentsForRun.map((item) => `- 📎 ${item.displayName}`).join("\n")}`
          : text,
    });
  }
  window.lunaScopeUi?.appendConversationEvent({
    id: "orchestration-thinking",
    type: "assistant_message",
    title: "LunaScope",
    summary: tr(
      "正在由 Orchestration Model 判断是否需要多 Agent，并生成 Worker 图…",
      "The Orchestration Model is deciding whether multiple Agents are useful and drafting the Worker graph…",
    ),
  });
  if (!isTauri()) {
    throw new Error("Browser preview cannot call the native Orchestrator.");
  }
  if (!providerConfigs.length) {
    providerConfigs = await invoke<ProviderConfig[]>("list_provider_configs");
  }
  nativeAgentPlans = {};
  const planningRunId = activeConversationRunId ?? `planning-${Date.now()}`;
  nativeOrchestrationSession =
    await invokeWithAllowOnce<OrchestrationSession>(
      "draft_native_orchestration",
      {
        objective: text,
        domainPackId: null,
        workspaceRoot: syncActiveWorkspace() || null,
        projectId: activeProjectId,
        threadId: activeConversationThreadId,
        messageId,
        attachmentIds: attachmentsForRun.map((item) => item.attachmentId),
        onProgress: createOrchestrationProgressChannel(planningRunId),
      },
      tr(
        "允许 Orchestration Model 使用已配置的 Provider 凭据分析本次对话吗？这可能产生少量 API 费用。",
        "Allow the Orchestration Model to use the configured Provider credential for this conversation? This may incur a small API charge.",
      ),
    );
  nativeProjectionPlan = nativeOrchestrationSession.plan;
  pendingAttachments = [];
  renderPendingAttachments();
  nativeOrchestrationResult = null;
  nativeChangeSets = [];
  nativeSourceRunIds = [nativeOrchestrationSession.runId];
  if (activeConversationThreadId) {
    window.lunaScopeUi?.bindConversationRun(
      activeConversationThreadId,
      nativeOrchestrationSession.runId,
    );
    activeConversationRunId = nativeOrchestrationSession.runId;
  }
  const plan = nativeOrchestrationSession.plan;
  if (activeConversationThreadId && plan.conversationTitle) {
    window.lunaScopeUi?.renameConversation(
      activeConversationThreadId,
      plan.conversationTitle,
    );
  }
  const roles = plan.workers.map((worker) => worker.role).join(" · ");
  window.lunaScopeUi?.appendConversationEvent({
    id: "orchestration-thinking",
    type: "assistant_message",
    title: "LunaScope",
    summary: tr(
      `已生成${plan.decision.kind === "multi_agent" ? "多 Agent" : "单 Agent"}编排：${roles}。${plan.decision.rationale}`,
      `${plan.decision.kind === "multi_agent" ? "Multi-agent" : "Single-agent"} orchestration created: ${roles}. ${plan.decision.rationale}`,
    ),
    openOrchestration: true,
  });
  await executeNativeOrchestration();
}

async function submitLiveGuidance(guidance: string): Promise<void> {
  const runId = nativeOrchestrationSession?.runId ?? activeConversationRunId;
  if (!runId || !activeConversationThreadId) {
    throw new Error("There is no active run to guide.");
  }
  const messageId = `message-${crypto.randomUUID()}`;
  window.lunaScopeUi?.appendConversationEvent({
    id: messageId,
    type: "user_message",
    title: "You",
    summary: guidance,
  });
  window.lunaScopeUi?.appendConversationEvent({
    id: `live-guidance-${runId}`,
    type: "assistant_commentary",
    title: tr("主 Agent · 正在重新规划", "Lead Agent · Replanning"),
    summary: tr(
      "已收到运行中引导。正在暂停新节点调度，由编排模型重新规划剩余链路；已完成的操作不会重跑。",
      "Guidance received. New dispatch is paused while the Orchestration Model replans the remaining path; completed operations will not rerun.",
    ),
  });
  let outcome: GuidanceReplanOutcome;
  try {
    outcome = await invoke<GuidanceReplanOutcome>(
      "guide_native_orchestration",
      {
        guidance,
        messageId,
        onProgress: createOrchestrationProgressChannel(runId),
      },
    );
  } catch (error) {
    if (errorMessage(error).includes("no orchestration is currently active")) {
      nativeOrchestrationRunning = false;
      nativeOrchestrationPaused = false;
      renderNativeRunControls();
      window.lunaScopeUi?.appendConversationEvent({
        id: `live-guidance-${runId}`,
        type: "assistant_commentary",
        title: tr("主 Agent · 开始下一轮", "Lead Agent · Starting next turn"),
        summary: tr(
          "上一轮恰好已经结束；这条消息将作为同一对话的新一轮任务继续执行。",
          "The previous run finished at the same moment, so this message is continuing as a new turn in the same conversation.",
        ),
      });
      await submitConversation(guidance, messageId);
      return;
    }
    throw error;
  }
  window.lunaScopeUi?.appendConversationEvent({
    id: `live-guidance-${runId}`,
    type: "assistant_commentary",
    title: tr("主 Agent · 已完成重新规划", "Lead Agent · Replan applied"),
    summary: `${localizedModelProse(outcome.guidance, "已根据新引导调整剩余执行链路。", "The remaining execution path was revised from the new guidance.")}\n\n${tr(`已引导 ${outcome.affectedWorkerIds.length} 个 Agent，其中 ${outcome.deferredWorkerIds.length} 个将在当前操作的安全边界后接续。`, `Guided ${outcome.affectedWorkerIds.length} Agent(s); ${outcome.deferredWorkerIds.length} will adopt it at the next safe boundary.`)}`,
  });
}

function createOrchestrationProgressChannel(
  displayRunId: string,
): Channel<OrchestrationProgress> {
  const channel = new Channel<OrchestrationProgress>();
  channel.onmessage = (event) => {
    if (event.plan && nativeOrchestrationSession) {
      nativeOrchestrationSession.plan = event.plan;
      nativeProjectionPlan = event.plan;
      nativeOrchestrationSession.snapshot.workers = Object.fromEntries(
        event.plan.workers.map((worker) => [worker.workerId, "queued"]),
      );
      renderNativeOrchestration();
    }
    const agentKey = event.workerId ?? "orchestrator";
    if (event.state === "plan_update" && event.agentPlan) {
      nativeAgentPlans[agentKey] = event.agentPlan;
      persistActiveOrchestrationState();
      renderNativeOrchestration();
      return;
    }
    if (event.state === "reasoning_summary") {
      renderReasoningSummaryProgress(displayRunId, event);
      return;
    }
    if (event.state === "model_commentary") {
      renderModelCommentaryProgress(displayRunId, event);
      return;
    }
    if (event.state === "tool_activity") {
      renderToolActivityProgress(displayRunId, event);
      return;
    }
    if (event.workerId && nativeOrchestrationSession) {
      nativeOrchestrationSession.snapshot.workers[event.workerId] =
        event.state as RuntimeSnapshot["workers"][string];
    }
    const role = orchestrationRoleLabel(event.role);
    const state = orchestrationStateLabel(event.state);
    const detail = orchestrationProgressDetail(event);
    window.lunaScopeUi?.appendConversationEvent({
      id: `agent-status-${displayRunId}-${event.workerId ?? event.role}`,
      type: "agent_status",
      title: tr(`子 Agent · ${role}`, `Agent · ${role}`),
      summary: `${state} · ${detail}`,
      openOrchestration: true,
      trackHistory: false,
    });
    window.lunaScopeUi?.setExecutionActivity({
      running: !["completed", "failed", "cancelled"].includes(event.state),
      workerId: event.workerId,
      role,
      state,
      detail,
    });
    updateNativeOrchestrationProgress(event);
  };
  return channel;
}

function renderModelCommentaryProgress(
  displayRunId: string,
  event: OrchestrationProgress,
): void {
  const itemId =
    event.itemId ?? `${event.workerId ?? "orchestrator"}-commentary`;
  const key = `${displayRunId}:${event.workerId ?? "orchestrator"}:${itemId}:commentary`;
  const previous = runtimeReasoningText.get(key) ?? "";
  const text =
    event.itemPhase === "delta"
      ? `${previous}${event.detail}`
      : event.detail.trim()
        ? event.detail
        : previous;
  runtimeReasoningText.set(key, text);
  if (!text.trim()) return;
  const role = event.workerId
    ? orchestrationRoleLabel(event.role)
    : tr("主 Agent", "Lead Agent");
  window.lunaScopeUi?.appendConversationEvent({
    id: `commentary-${key}`,
    type: "assistant_commentary",
    title: tr(`${role} · 思考与下一步`, `${role} · Analysis and next action`),
    summary: text,
    openOrchestration: false,
  });
  window.lunaScopeUi?.setExecutionActivity({
    running: event.itemPhase !== "completed",
    workerId: event.workerId,
    role,
    state: tr("正在分析", "Analyzing"),
    detail: text,
  });
}

function renderToolActivityProgress(
  displayRunId: string,
  event: OrchestrationProgress,
): void {
  const itemId =
    event.itemId ?? `${event.workerId ?? "orchestrator"}-${event.tool ?? "tool"}`;
  const role = event.workerId
    ? orchestrationRoleLabel(event.role)
    : tr("主 Agent", "Lead Agent");
  const failed = event.itemPhase === "failed";
  const completed = event.itemPhase === "completed";
  window.lunaScopeUi?.appendConversationEvent({
    id: `tool-${displayRunId}-${event.workerId ?? "orchestrator"}-${itemId}`,
    type: "tool_activity",
    title: `${role} · ${event.tool ?? tr("工具", "Tool")}`,
    summary: `${failed ? tr("失败", "Failed") : completed ? tr("完成", "Completed") : tr("运行中", "Running")} · ${event.detail}`,
    openOrchestration: failed,
  });
  window.lunaScopeUi?.setExecutionActivity({
    running: !failed && !completed,
    workerId: event.workerId,
    role,
    state: failed
      ? tr("工具失败", "Tool failed")
      : completed
        ? tr("工具完成", "Tool completed")
        : tr("正在运行工具", "Running tool"),
    detail: event.detail,
  });
}

function renderReasoningSummaryProgress(
  displayRunId: string,
  event: OrchestrationProgress,
): void {
  const itemId =
    event.itemId ??
    `${event.workerId ?? "orchestrator"}-${event.summaryIndex ?? 0}`;
  const key = `${displayRunId}:${event.workerId ?? "orchestrator"}:${itemId}:${event.summaryIndex ?? 0}`;
  const previous = runtimeReasoningText.get(key) ?? "";
  const text =
    event.itemPhase === "delta"
      ? `${previous}${event.detail}`
      : event.itemPhase === "completed" && event.detail.trim()
        ? event.detail
        : previous;
  runtimeReasoningText.set(key, text);
  if (!text.trim()) return;
  const role = event.workerId
    ? orchestrationRoleLabel(event.role)
    : tr("主 Agent", "Lead Agent");
  const providerSummary = event.summarySource === "provider";
  window.lunaScopeUi?.appendConversationEvent({
    id: `reasoning-${key}`,
    type: "reasoning_summary",
    title: providerSummary
      ? tr(`${role} · 推理摘要`, `${role} · Reasoning summary`)
      : tr(`${role} · 行动说明`, `${role} · Action update`),
    summary: text,
    openOrchestration: false,
  });
  window.lunaScopeUi?.setExecutionActivity({
    running: event.itemPhase !== "completed",
    workerId: event.workerId,
    role,
    state: providerSummary
      ? tr("正在推理", "Reasoning")
      : tr("正在规划下一步", "Planning next action"),
    detail: text,
  });
}

function updateNativeOrchestrationProgress(event: OrchestrationProgress): void {
  if (!event.workerId) return;
  const running = [
    "running_model",
    "running_tool",
    "verifying",
    "recovering",
    "retrying",
    "localizing",
    "repair_planning",
    "replanning",
    "context_compaction",
  ].includes(event.state);
  const selector = `[data-native-worker-id="${CSS.escape(event.workerId)}"]`;
  document.querySelectorAll<HTMLElement>(selector).forEach((node) => {
    node.dataset.workerState = event.state;
    node.classList.toggle("is-running", running);
    const status = node.querySelector<HTMLElement>(".node-status");
    if (status) status.textContent = orchestrationStateLabel(event.state);
    const runtime = node.querySelector<HTMLElement>("[data-runtime-detail]");
    if (runtime) runtime.textContent = orchestrationProgressDetail(event);
  });
}

function orchestrationRoleLabel(role: string): string {
  const labels: Record<string, [string, string]> = {
    builder: ["项目实现", "Builder"],
    frontend: ["界面与交互", "Frontend"],
    backend: ["后端实现", "Backend"],
    planner: ["需求与方案", "Planner"],
    researcher: ["资料研究", "Researcher"],
    reviewer: ["代码审查", "Reviewer"],
    verifier: ["独立验收", "Verifier"],
    documentation: ["文档整理", "Documentation"],
    game_designer: ["游戏设计", "Game designer"],
    academic_writer: ["学术写作", "Academic writer"],
  };
  const label = labels[role.toLowerCase()];
  return label
    ? tr(label[0], label[1])
    : tr(`自定义子 Agent（${role}）`, role);
}

function localizedModelProse(
  value: string,
  chineseFallback: string,
  englishFallback = "",
): string {
  const text = value.trim();
  if (userPreferences.language === "english") {
    return text || englishFallback || chineseFallback;
  }
  return /[\u3400-\u9fff]/u.test(text) ? text : chineseFallback;
}

function orchestrationStateLabel(state: string): string {
  const labels: Record<string, [string, string]> = {
    queued: ["等待调度", "Queued"],
    waiting_dependency: ["等待前置结果", "Waiting for dependencies"],
    running_model: ["正在分析与生成", "Analyzing and generating"],
    running_tool: ["正在执行操作", "Running an operation"],
    recovering: ["正在自动恢复", "Recovering automatically"],
    localizing: ["正在统一回复语言", "Localizing output"],
    repair_planning: ["正在生成自动修复链路", "Generating repair chain"],
    verifying: ["正在验收", "Verifying"],
    retrying: ["正在重试失败节点", "Retrying failed node"],
    completed: ["已完成", "Completed"],
    failed: ["执行失败", "Failed"],
    cancelled: ["已取消", "Cancelled"],
    paused: ["已暂停", "Paused"],
  };
  const label = labels[state.toLowerCase()];
  return label
    ? tr(label[0], label[1])
    : state.replaceAll("_", " ").toUpperCase();
}

function orchestrationToolLabel(tool: string): string {
  const labels: Record<string, [string, string]> = {
    list_files: ["检查项目文件清单", "Inspecting the project file list"],
    read_file: ["读取项目文件", "Reading a project file"],
    search_text: ["搜索项目内容", "Searching project content"],
    write_file: [
      "写入隔离副本并立即同步到项目",
      "Writing to the isolated copy and synchronizing to the project immediately",
    ],
    replace_in_file: [
      "修改隔离副本并立即同步到项目",
      "Editing the isolated copy and synchronizing to the project immediately",
    ],
    apply_patch: [
      "原子应用文件补丁并同步到项目",
      "Applying an atomic file patch and synchronizing it to the project",
    ],
    run_process: ["运行有限时长的检查命令", "Running a bounded check command"],
  };
  const label = labels[tool];
  return label ? tr(label[0], label[1]) : tool;
}

function orchestrationProgressDetail(event: OrchestrationProgress): string {
  if (event.tool) {
    const marker = `Executing ${event.tool} · `;
    const target = event.detail.startsWith(marker)
      ? event.detail.slice(marker.length).trim()
      : "";
    return `${orchestrationToolLabel(event.tool)}${target ? ` · \`${target}\`` : ""}`;
  }
  const detail = event.detail;
  const streamed = detail.match(
    /Model response is streaming \((\d+) characters received\)/,
  );
  if (streamed) {
    return tr(
      `正在生成本节点结果 · 已接收 ${streamed[1]} 个字符`,
      `Generating this node's result · ${streamed[1]} characters received`,
    );
  }
  const step = detail.match(/Model is evaluating tool results \(step (\d+)\)/);
  if (step) {
    return tr(
      `正在检查第 ${step[1]} 步操作结果并决定下一步`,
      `Reviewing operation ${step[1]} and choosing the next action`,
    );
  }
  const known: Record<string, [string, string]> = {
    "Model is inspecting the assignment and workspace": [
      "正在读取任务、项目文件与前置结果",
      "Reading the assignment, project files, and dependency results",
    ],
    "Workspace changes exist; requesting the final structured result": [
      "文件已同步到项目，正在整理本节点证据",
      "Files are synchronized to the project; collecting evidence for this node",
    ],
    "No workspace change was observed; requesting a corrective tool step": [
      "尚未检测到文件变更，正在要求 Agent 实际执行写入",
      "No file change was detected; requiring the Agent to perform the write",
    ],
    "A malformed provider response was detected; retrying the current step without rerunning completed tools":
      [
        "模型响应格式异常；保留已完成操作，只重试当前步骤",
        "The provider response was malformed; preserving completed operations and retrying only this step",
      ],
    "Verification evidence is complete; repairing only the final structured result":
      [
        "验收操作已完成；仅修复最终结果格式，不重复执行检查",
        "Verification work is complete; repairing only the final result format",
      ],
    "Tool evidence is sufficient; settling this Worker before the execution budget is exhausted":
      [
        "工具与工作区证据已经足够，正在结束本节点并整理交付",
        "Tool and workspace evidence are sufficient; settling this node",
      ],
    "Queued for dispatch": ["已进入调度队列", "Ready for dispatch"],
    "Waiting for dependency artifacts": [
      "等待前置 Agent 的结果",
      "Waiting for dependency results",
    ],
    "Retrying only this failed Agent": [
      "仅重新运行这个失败的 Agent",
      "Retrying only this failed Agent",
    ],
    "Resuming after the failed dependency recovers": [
      "等待失败的前置节点恢复后继续",
      "Waiting for the failed dependency to recover",
    ],
  };
  const label = known[detail];
  if (label) return tr(label[0], label[1]);
  if (userPreferences.language === "english") return detail;
  if (/[\u3400-\u9fff]/u.test(detail)) return detail;
  return orchestrationStateLabel(event.state);
}

function failedWorkerIds(result: OrchestrationRunResult | null): string[] {
  if (!result) return [];
  return Object.values(result.workers)
    .filter((worker) => worker.state === "failed")
    .map((worker) => worker.workerId);
}

async function automaticallyRetryFailedAgents(
  workspaceRoot: string,
  displayRunId: string,
): Promise<{ cycles: number; lastError: string | null }> {
  let cycles = 0;
  let lastError: string | null = null;
  while (cycles < automaticFailedWorkerRetryLimit) {
    const previous = nativeOrchestrationResult;
    const failed = failedWorkerIds(previous);
    const sourceRunId = nativeRetrySourceRunId;
    if (!previous || failed.length === 0 || !sourceRunId) break;
    cycles += 1;
    window.lunaScopeUi?.appendConversationEvent({
      id: `automatic-retry-${displayRunId}`,
      type: "agent_status",
      title: tr("主 Agent · 自动恢复", "Lead Agent · Automatic recovery"),
      summary: tr(
        `检测到 ${failed.length} 个失败节点，正在执行第 ${cycles}/${automaticFailedWorkerRetryLimit} 轮局部恢复；已完成节点不会重跑。`,
        `Detected ${failed.length} failed node(s). Running local recovery ${cycles}/${automaticFailedWorkerRetryLimit}; completed nodes will not rerun.`,
      ),
      openOrchestration: true,
      trackHistory: false,
    });
    try {
      const outcome = await invoke<RetryOrchestrationOutcome>(
        "retry_failed_native_orchestration",
        {
          sourceRunId,
          workspaceRoot,
          accessMode: computerAccessMode,
          allowOnce: true,
          onProgress: createOrchestrationProgressChannel(displayRunId),
        },
      );
      nativeRetrySourceRunId = outcome.runId;
      localStorage.setItem(retrySourceRunStorageKey, outcome.runId);
      nativeSourceRunIds = [
        ...new Set([...nativeSourceRunIds, outcome.runId]),
      ];
      nativeOrchestrationResult = mergeRetryResult(previous, outcome.result);
      if (nativeOrchestrationSession) {
        for (const worker of Object.values(outcome.result.workers)) {
          nativeOrchestrationSession.snapshot.workers[worker.workerId] =
            worker.state;
        }
      }
      renderNativeOrchestrationResult();
    } catch (error) {
      lastError = errorMessage(error);
      break;
    }
  }
  return { cycles, lastError };
}

function verificationHasFatalDefects(
  result: OrchestrationRunResult | null,
): boolean {
  if (!result) return false;
  return (
    result.verification.status === "failed_verification" ||
    result.verification.findings.some(
      (finding) => finding.severity === "fatal",
    )
  );
}

function verificationStatusLabel(
  status: OrchestrationRunResult["verification"]["status"],
): string {
  const labels = {
    verified: tr("已验证", "Verified"),
    partially_verified: tr("部分验证", "Partially verified"),
    unverified: tr("未验证", "Unverified"),
    unable_to_verify: tr("无法验证", "Unable to verify"),
    failed_verification: tr("验收失败", "Failed verification"),
  };
  return labels[status];
}

function mergeRetryResult(
  previous: OrchestrationRunResult,
  retry: OrchestrationRunResult,
): OrchestrationRunResult {
  const workers = { ...previous.workers, ...retry.workers };
  const artifacts = [...previous.artifacts, ...retry.artifacts].filter(
    (artifact, index, values) =>
      values.findIndex(
        (candidate) => candidate.artifactId === artifact.artifactId,
      ) === index,
  );
  const handoffs = [...previous.handoffs, ...retry.handoffs].filter(
    (handoff, index, values) =>
      values.findIndex(
        (candidate) => candidate.handoffId === handoff.handoffId,
      ) === index,
  );
  return {
    ...previous,
    workers,
    stateChanges: [...previous.stateChanges, ...retry.stateChanges],
    artifacts,
    handoffs,
    synthesis: Object.values(workers)
      .filter((worker) => worker.state === "completed" && worker.summary.trim())
      .map((worker) => `${worker.workerId}: ${worker.summary}`)
      .join("\n"),
    verification: retry.verification,
  };
}

async function refreshNativeChangeSets(runIds: string[]): Promise<void> {
  const unique = [...new Set(runIds.filter(Boolean))];
  if (!isTauri() || unique.length === 0) {
    nativeChangeSets = [];
    return;
  }
  const settled = await Promise.allSettled(
    unique.map((runId) =>
      invoke<OrchestrationChangeSet>("read_orchestration_change_set", {
        runId,
      }),
    ),
  );
  nativeChangeSets = settled.flatMap((result) =>
    result.status === "fulfilled" ? [result.value] : [],
  );
  renderNativeOrchestration();
}

async function executeNativeOrchestration(): Promise<void> {
  const session = nativeOrchestrationSession;
  if (!session || nativeOrchestrationRunning) return;
  const workspaceRoot = syncActiveWorkspace();
  if (!workspaceRoot) {
    throw new Error(
      tr(
        "当前项目没有可执行的 workspace 文件夹。",
        "The current project has no executable workspace folder.",
      ),
    );
  }
  nativeOrchestrationRunning = true;
  nativeOrchestrationPaused = false;
  renderNativeRunControls();
  renderNativeOrchestration();
  window.lunaScopeUi?.appendConversationEvent({
    id: `orchestration-run-${session.runId}`,
    type: "assistant_message",
    title: "LunaScope",
    summary: tr(
      `编排图已确认，正在自动调度 ${session.plan.workers.length} 个 Agent。`,
      `Graph confirmed. Automatically dispatching ${session.plan.workers.length} Agent(s).`,
    ),
    openOrchestration: true,
  });
  for (const worker of session.plan.workers) {
    window.lunaScopeUi?.appendConversationEvent({
      id: `agent-status-${session.runId}-${worker.workerId}`,
      type: "agent_status",
      title: tr(
        `子 Agent · ${orchestrationRoleLabel(worker.role)}`,
        `Agent · ${orchestrationRoleLabel(worker.role)}`,
      ),
      summary: `${orchestrationStateLabel("queued")} · ${worker.task}`,
      openOrchestration: true,
    });
  }
  let autonomousRepairCycles = 0;
  try {
    const automatic =
      bypassMode ||
      computerAccessMode === "self_approve" ||
      computerAccessMode === "full_access";
    const invokeRun = (allowOnce: boolean, onProgress: Channel<OrchestrationProgress>) =>
      invoke<NativeOrchestrationOutcome>("run_native_orchestration", {
        runId: session.runId,
        workspaceRoot,
        accessMode: computerAccessMode,
        onProgress,
        allowOnce,
      });
    if (automatic) {
      const outcome = await invokeRun(
        true,
        createOrchestrationProgressChannel(session.runId),
      );
      nativeOrchestrationSession = outcome.session;
      nativeProjectionPlan = outcome.session.plan;
      nativeOrchestrationResult = outcome.result;
      nativeSourceRunIds = outcome.sourceRunIds;
      autonomousRepairCycles = outcome.repairCycles;
    } else {
      try {
        const outcome = await invokeRun(
          false,
          createOrchestrationProgressChannel(session.runId),
        );
        nativeOrchestrationSession = outcome.session;
        nativeProjectionPlan = outcome.session.plan;
        nativeOrchestrationResult = outcome.result;
        nativeSourceRunIds = outcome.sourceRunIds;
        autonomousRepairCycles = outcome.repairCycles;
      } catch (error) {
        const message = errorMessage(error);
        if (!message.includes("APPROVAL_REQUIRED")) throw error;
        const confirmation = tr(
          `允许 ${session.plan.workers.length} 个 Agent 在隔离工作区中读取、修改文件并运行验证，然后把通过检查的改动写回项目吗？`,
          `Allow ${session.plan.workers.length} Agent(s) to read and edit files in isolated workspaces, run verification, and apply validated changes back to the project?`,
        );
        if (!window.confirm(`${confirmation}\n\n${message}`)) {
          throw new Error("用户未授予本次操作权限");
        }
        const outcome = await invokeRun(
          true,
          createOrchestrationProgressChannel(session.runId),
        );
        nativeOrchestrationSession = outcome.session;
        nativeProjectionPlan = outcome.session.plan;
        nativeOrchestrationResult = outcome.result;
        nativeSourceRunIds = outcome.sourceRunIds;
        autonomousRepairCycles = outcome.repairCycles;
      }
    }
    nativeRetrySourceRunId = nativeOrchestrationSession?.runId ?? session.runId;
    localStorage.setItem(retrySourceRunStorageKey, nativeRetrySourceRunId);
    const automaticRecovery = await automaticallyRetryFailedAgents(
      workspaceRoot,
      session.runId,
    );
    autonomousRepairCycles += automaticRecovery.cycles;
    await refreshNativeChangeSets(nativeSourceRunIds);
    renderNativeOrchestrationResult();
    const failed = failedWorkerIds(nativeOrchestrationResult);
    const fatalVerification = verificationHasFatalDefects(
      nativeOrchestrationResult,
    );
    if (failed.length > 0 || fatalVerification) {
      window.lunaScopeUi?.appendConversationEvent({
        id: `assistant-${nativeRetrySourceRunId ?? session.runId}`,
        type: "assistant_message",
        title: "LunaScope",
        summary: fatalVerification
          ? tr(
              `独立验收仍检测到致命缺陷。LunaScope 已自动执行 ${autonomousRepairCycles} 轮修复并保留最后一条修复链路；本次任务不会被标记为完成。`,
              `Independent verification still found a fatal defect after ${autonomousRepairCycles} autonomous repair cycle(s). The final repair graph remains available and the task is not marked complete.`,
            )
          : tr(
              `${failed.length} 个子 Agent 在自动局部恢复后仍然失败。是否再次只重试这些失败节点？已完成节点不会重跑，被依赖失败取消的下游会在恢复后继续。${automaticRecovery.lastError ? ` 自动恢复错误：${automaticRecovery.lastError}` : ""}`,
              `${failed.length} child Agent(s) remain failed after automatic local recovery. Retry only those failed Agent(s) again? Completed nodes will not rerun; dependency-cancelled descendants will resume after recovery.`,
            ),
        openOrchestration: true,
        retryConversation: failed.length > 0,
      });
    } else {
      window.lunaScopeUi?.appendConversationEvent({
        id: `assistant-${nativeRetrySourceRunId ?? session.runId}`,
        type: "assistant_message",
        title: "LunaScope",
        summary: `${tr("所有 Agent 已结束。", "All Agents finished.")} ${localizedModelProse(nativeOrchestrationResult.synthesis, "所有节点已完成，详细证据保存在交付物中。", "All nodes completed with stored evidence.")} · ${tr("验证", "Verification")}: ${verificationStatusLabel(nativeOrchestrationResult.verification.status)}`,
        openOrchestration: true,
      });
      window.setTimeout(clearCompletedOrchestrationGraph, 0);
    }
  } catch (error) {
    window.lunaScopeUi?.appendConversationEvent({
      id: `orchestration-run-${session.runId}`,
      type: "assistant_message",
      title: "LunaScope",
      summary: `${tr("编排执行失败：", "Orchestration execution failed: ")}${errorMessage(error)}`,
      openOrchestration: true,
      retryConversation: false,
    });
  } finally {
    nativeOrchestrationRunning = false;
    nativeOrchestrationPaused = false;
    renderNativeRunControls();
    window.lunaScopeUi?.setExecutionActivity({
      running: false,
      workerId: null,
      role: "LunaScope",
      state: tr("已结束", "Finished"),
      detail: nativeOrchestrationResult
        ? tr("运行结果已写入工作区并完成汇总", "Workspace result recorded and synthesized")
        : tr("运行已停止", "Run stopped"),
    });
  }
}

async function retryFailedAgents(sourceRunIdOverride?: string): Promise<void> {
  const session = nativeOrchestrationSession;
  const previous = nativeOrchestrationResult;
  const sourceRunId = sourceRunIdOverride ?? nativeRetrySourceRunId;
  const workspaceRoot = syncActiveWorkspace();
  if (!sourceRunId || !workspaceRoot) return;
  const failed = previous ? failedWorkerIds(previous) : [];
  if (previous && failed.length === 0) {
    throw new Error(
      tr(
        "当前结果中没有可局部重试的失败子 Agent。",
        "The current result has no failed child Agent eligible for a local retry.",
      ),
    );
  }

  nativeOrchestrationRunning = true;
  nativeOrchestrationPaused = false;
  renderNativeRunControls();
  renderNativeOrchestration();
  window.lunaScopeUi?.appendConversationEvent({
    id: `orchestration-run-${session?.runId ?? sourceRunId}`,
    type: "assistant_message",
    title: "LunaScope",
    summary: tr(
      `已确认局部重试：只重跑 ${failed.length} 个失败子 Agent；已完成节点保持不动。`,
      `Local retry confirmed: rerunning only ${failed.length} failed child Agent(s); completed nodes remain untouched.`,
    ),
    openOrchestration: true,
  });
  try {
    const outcome = await invoke<RetryOrchestrationOutcome>(
      "retry_failed_native_orchestration",
      {
        sourceRunId,
        workspaceRoot,
        accessMode: computerAccessMode,
        allowOnce: true,
        onProgress: createOrchestrationProgressChannel(
          session?.runId ?? sourceRunId,
        ),
      },
    );
    nativeRetrySourceRunId = outcome.runId;
    localStorage.setItem(retrySourceRunStorageKey, outcome.runId);
    nativeSourceRunIds = [...new Set([...nativeSourceRunIds, outcome.runId])];
    nativeOrchestrationResult = previous
      ? mergeRetryResult(previous, outcome.result)
      : outcome.result;
    if (session) {
      for (const worker of Object.values(outcome.result.workers)) {
        session.snapshot.workers[worker.workerId] = worker.state;
      }
    }
    renderNativeOrchestration();
    renderNativeOrchestrationResult();
    await refreshNativeChangeSets(nativeSourceRunIds);
    const stillFailed = failedWorkerIds(nativeOrchestrationResult);
    const resumed = outcome.resumedWorkerIds.length;
    if (stillFailed.length > 0) {
      window.lunaScopeUi?.appendConversationEvent({
        id: `assistant-${outcome.runId}`,
        type: "assistant_message",
        title: "LunaScope",
        summary: tr(
          `局部重试后仍有 ${stillFailed.length} 个子 Agent 失败。是否再次只重试失败节点？`,
          `${stillFailed.length} child Agent(s) still failed after local recovery. Retry only those failed nodes again?`,
        ),
        openOrchestration: true,
        retryConversation: true,
      });
    } else {
      window.lunaScopeUi?.appendConversationEvent({
        id: `assistant-${outcome.runId}`,
        type: "assistant_message",
        title: "LunaScope",
        summary: tr(
          `局部恢复完成：${outcome.retriedWorkerIds.length} 个失败节点已重试，${resumed} 个依赖下游已继续；未重跑任何已完成节点。`,
          `Local recovery completed: ${outcome.retriedWorkerIds.length} failed node(s) retried and ${resumed} dependent node(s) resumed; no completed node reran.`,
        ),
        openOrchestration: true,
      });
      window.setTimeout(clearCompletedOrchestrationGraph, 0);
    }
  } catch (error) {
    window.lunaScopeUi?.appendConversationEvent({
      id: `orchestration-run-${session?.runId ?? sourceRunId}`,
      type: "assistant_message",
      title: "LunaScope",
      summary: `${tr("局部重试失败：", "Local retry failed: ")}${errorMessage(error)}`,
      openOrchestration: true,
      retryConversation: true,
    });
  } finally {
    nativeOrchestrationRunning = false;
    nativeOrchestrationPaused = false;
    renderNativeRunControls();
    window.lunaScopeUi?.setExecutionActivity({
      running: false,
      workerId: null,
      role: "LunaScope",
      state: tr("已结束", "Finished"),
      detail: tr("局部重试链路已结束", "Local retry chain finished"),
    });
  }
}

function showSlashMenu(): void {
  const box = document.querySelector<HTMLElement>("#composer .composer-box");
  if (!box) return;
  box.querySelector("#nativeSlashMenu")?.remove();
  box.insertAdjacentHTML(
    "afterbegin",
    `<div class="slash-menu" id="nativeSlashMenu">
      ${[
        ["/ultranote", tr("在当前对话生成智能笔记（可添加文件与图片）", "Create smart notes in this conversation with files and images")],
        ["/project", tr("打开项目管理", "Open project management")],
        ["/orchestrate", tr("打开多 Agent 编排", "Open multi-agent orchestration")],
        ["/checkpoint", tr("请求运行时检查点", "Request a runtime checkpoint")],
        ["/verify", tr("运行验证流程", "Run verification")],
      ]
        .map(
          ([command, description]) =>
            `<button type="button" data-native-action="slash-command" data-command="${command}"><code>${command}</code><span>${description}</span></button>`,
        )
        .join("")}
    </div>`,
  );
}

function handleSlashCommand(command: string): boolean {
  const normalized = command.trim().toLowerCase();
  document.querySelector("#nativeSlashMenu")?.remove();
  if (normalized === "/ultranote") {
    return false;
  }
  if (normalized === "/project") {
    void openProjectManager(false);
    return true;
  }
  if (normalized === "/orchestrate") {
    document
      .querySelector<HTMLButtonElement>('button[data-view="orchestration"]')
      ?.click();
    return true;
  }
  if (normalized === "/verify") {
    document
      .querySelector<HTMLButtonElement>('[data-v14="run-verification"]')
      ?.click();
    return true;
  }
  if (normalized === "/checkpoint") {
    document
      .querySelector<HTMLButtonElement>('[data-v14="checkpoint"]')
      ?.click();
    return true;
  }
  return false;
}

async function handleProjectAction(
  button: HTMLButtonElement,
): Promise<boolean> {
  const action = button.dataset.nativeAction ?? "";
  if (
    !action.startsWith("project-") &&
    action !== "slash-command"
  ) {
    return false;
  }
  if (action === "slash-command") {
    const command = button.dataset.command ?? "";
    if (!handleSlashCommand(command)) {
      const input =
        document.querySelector<HTMLTextAreaElement>("#composerInput");
      if (input) {
        input.value = command;
        input.focus();
      }
    }
    return true;
  }
  if (action === "project-close") {
    ensureProjectDialog().close();
    return true;
  }
  if (action === "project-new") {
    editingProjectId = null;
    projectDraftName = "";
    projectDraftKind = "general";
    projectDraftCourseTitle = "";
    projectDraftCourseCode = "";
    projectDraftSyllabus = null;
    projectDraftFolders = [];
    projectWorkspacePath = "";
    renderProjectDialog();
    return true;
  }
  if (action === "project-edit") {
    const project = projects.find(
      (item) => item.projectId === button.dataset.projectId,
    );
    if (project) {
      activeProjectId = project.projectId;
      editingProjectId = project.projectId;
      projectDraftName = project.name;
      projectDraftKind = project.kind;
      projectDraftCourseTitle = project.ultranote?.courseTitle ?? "";
      projectDraftCourseCode = project.ultranote?.courseCode ?? "";
      projectDraftSyllabus = project.ultranote
        ? {
            attachmentId: project.ultranote.syllabusAttachmentId,
            displayName: project.ultranote.syllabusDisplayName,
            mediaType: "application/octet-stream",
            kind: "document",
            sizeBytes: 0,
            extractedCharacters: 0,
            embeddedImageCount: 0,
            previewPath: null,
            extractionWarning: null,
          }
        : null;
      projectDraftFolders = project.folders.map((folder) => folder.path);
      projectWorkspacePath =
        project.folders.find((folder) => folder.isWorkspace)?.path ?? "";
      renderProjectDialog();
    }
    return true;
  }
  if (action === "project-delete") {
    const projectId = button.dataset.projectId ?? editingProjectId ?? "";
    const project = projects.find((item) => item.projectId === projectId);
    if (!project) return true;
    if (nativeOrchestrationRunning && projectId === activeProjectId) {
      window.alert(
        tr(
          "运行中的项目暂不能删除。请先停止运行。",
          "A project with an active run cannot be deleted. Stop the run first.",
        ),
      );
      return true;
    }
    if (
      !window.confirm(
        tr(
          `删除项目“${project.name}”及其对话记录？不会删除项目文件夹中的本地文件。此操作无法撤销。`,
          `Delete project “${project.name}” and its conversations? Local files inside the project folders will not be deleted. This cannot be undone.`,
        ),
      )
    ) {
      return true;
    }
    try {
      if (isTauri()) {
        await invoke<boolean>("delete_project", { projectId });
        projects = await invoke<LunaProject[]>("list_projects");
      } else {
        projects = projects.filter((item) => item.projectId !== projectId);
      }
      const removal =
        window.lunaScopeUi?.removeProjectConversations(projectId) ?? null;
      for (const threadId of removal?.removedThreadIds ?? []) {
        orchestrationByThread.delete(threadId);
      }
      if (projectId === activeProjectId) {
        activeProjectId = projects[0]?.projectId ?? null;
        applyActiveConversationReference(
          removal?.activeConversation ?? null,
          true,
        );
      }
      const next =
        projects.find((item) => item.projectId === activeProjectId) ??
        projects[0] ??
        null;
      activeProjectId = next?.projectId ?? null;
      editingProjectId = next?.projectId ?? null;
      projectDraftName = next?.name ?? "";
      projectDraftFolders = next?.folders.map((folder) => folder.path) ?? [];
      projectWorkspacePath =
        next?.folders.find((folder) => folder.isWorkspace)?.path ?? "";
      activeWorkspaceRoot = projectWorkspacePath;
      renderProjectDialog();
      applyUiLanguage();
      setSettingsStatus(
        "projectActionStatus",
        tr("项目已删除，本地文件未改动。", "Project deleted. Local files were not changed."),
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "projectActionStatus",
        `${tr("删除失败：", "Delete failed: ")}${errorMessage(error)}`,
        "error",
      );
    }
    return true;
  }
  if (action === "project-remove-folder") {
    captureProjectDraftFields();
    const index = Number(button.dataset.folderIndex);
    const removed = projectDraftFolders[index];
    projectDraftFolders.splice(index, 1);
    if (removed === projectWorkspacePath) {
      projectWorkspacePath = projectDraftFolders[0] ?? "";
    }
    renderProjectDialog();
    return true;
  }
  if (action === "project-select-folders") {
    captureProjectDraftFields();
    if (!isTauri()) {
      projectDraftFolders = [
        "D:\\Projects\\TinyApp",
        "D:\\Projects\\SharedReferences",
      ];
      projectWorkspacePath = projectDraftFolders[0];
      renderProjectDialog();
      return true;
    }
    const selection = await open({
      directory: true,
      multiple: true,
      title: tr("选择项目文件夹", "Choose project folders"),
    });
    const selected = !selection
      ? []
      : Array.isArray(selection)
        ? selection
        : [selection];
    projectDraftFolders = [
      ...new Set([...projectDraftFolders, ...selected]),
    ];
    if (!projectWorkspacePath) {
      projectWorkspacePath = projectDraftFolders[0] ?? "";
    }
    renderProjectDialog();
    return true;
  }
  if (action === "project-select-syllabus") {
    captureProjectDraftFields();
    if (!isTauri()) {
      projectDraftSyllabus = {
        attachmentId: "attachment-preview",
        displayName: "syllabus.pdf",
        mediaType: "application/pdf",
        kind: "pdf",
        sizeBytes: 0,
        extractedCharacters: 0,
        embeddedImageCount: 0,
        previewPath: null,
        extractionWarning: null,
      };
      renderProjectDialog();
      return true;
    }
    const selection = await open({
      directory: false,
      multiple: false,
      title: tr("选择课程大纲", "Choose syllabus"),
      filters: [{
        name: "Course documents",
        extensions: ["pdf", "doc", "docx", "ppt", "pptx", "xls", "xlsx", "md", "txt"],
      }],
    });
    if (typeof selection === "string") {
      const imported = await invoke<ImportedAttachment[]>("import_attachments", {
        request: { paths: [selection] },
      });
      projectDraftSyllabus = imported[0] ?? null;
      renderProjectDialog();
    }
    return true;
  }
  if (action === "project-save") {
    const name =
      document.querySelector<HTMLInputElement>("#nativeProjectName")?.value ??
      "";
    projectDraftName = name;
    if (!isTauri()) {
      const now = new Date().toISOString();
      const project: LunaProject = {
        projectId: editingProjectId ?? "project-preview",
        name: name || "Preview project",
        folders: projectDraftFolders.map((path, index) => ({
          folderId: `folder-${index}`,
          path,
          displayName: path.split("\\").pop() ?? path,
          isWorkspace: path === projectWorkspacePath,
        })),
        kind: projectDraftKind,
        ultranote: projectDraftKind === "ultra_note" && projectDraftSyllabus
          ? {
              courseId: "course-preview",
              courseTitle: projectDraftCourseTitle,
              courseCode: projectDraftCourseCode || null,
              syllabusAttachmentId: projectDraftSyllabus.attachmentId,
              syllabusDisplayName: projectDraftSyllabus.displayName,
            }
          : null,
        createdAt: now,
        updatedAt: now,
      };
      projects = [project];
      activeProjectId = project.projectId;
      renderProjectDialog();
      applyUiLanguage();
      return true;
    }
    try {
      if (projectDraftKind === "ultra_note" && !projectDraftSyllabus) {
        throw new Error(tr("请选择课程大纲。", "Choose a syllabus file."));
      }
      const project = await invokeWithAllowOnce<LunaProject>(
        "save_project",
        {
          request: {
            projectId: editingProjectId,
            name,
            folders: projectDraftFolders.map((path) => ({
              path,
              isWorkspace: path === projectWorkspacePath,
            })),
            kind: projectDraftKind,
            ultranote: projectDraftKind === "ultra_note" && projectDraftSyllabus
              ? {
                  courseTitle: projectDraftCourseTitle,
                  courseCode: projectDraftCourseCode || null,
                  syllabusAttachmentId: projectDraftSyllabus.attachmentId,
                  syllabusDisplayName: projectDraftSyllabus.displayName,
                }
              : null,
          },
        },
        tr(
          "允许编排模型解析课程大纲并写入课程基础信息吗？",
          "Allow the Orchestration model to parse the syllabus into the course foundation?",
        ),
      );
      projects = await invoke<LunaProject[]>("list_projects");
      activeProjectId = project.projectId;
      editingProjectId = project.projectId;
      projectDraftName = project.name;
      projectDraftKind = project.kind;
      projectDraftCourseTitle = project.ultranote?.courseTitle ?? "";
      projectDraftCourseCode = project.ultranote?.courseCode ?? "";
      projectDraftFolders = project.folders.map((folder) => folder.path);
      projectWorkspacePath =
        project.folders.find((folder) => folder.isWorkspace)?.path ?? "";
      activeWorkspaceRoot = projectWorkspacePath;
      applyUiLanguage();
      ensureProjectDialog().close();
      await createNewConversation();
    } catch (error) {
      setSettingsStatus(
        "projectActionStatus",
        errorMessage(error),
        "error",
      );
    }
    return true;
  }
  return true;
}

function currentThreadId(): string {
  return (
    location.hash.match(/\/thread\/([^/]+)/)?.[1] ??
    ultraNoteWorkspace?.threadId ??
    "rebuild"
  );
}

function ultraNoteStatus(message: string, kind = ""): void {
  const element = document.querySelector<HTMLElement>("#ultraNoteStatus");
  if (!element) return;
  element.className = `settings-status ${kind}`;
  element.textContent = message;
}

function sourceKindOptions(): string {
  const kinds: Array<[CourseSourceKind, string, string]> = [
    ["slides", "幻灯片", "Slides"],
    ["lecture_notes", "课堂笔记", "Lecture notes"],
    ["transcript", "转录文本", "Transcript"],
    ["reading", "阅读材料", "Reading"],
    ["textbook_pages", "教材页", "Textbook pages"],
    ["board_photo", "板书照片 / 提取文本", "Board photo / extracted text"],
    ["lab_material", "实验材料", "Lab material"],
    ["code", "代码", "Code"],
    ["dataset", "数据集", "Dataset"],
  ];
  return kinds
    .map(
      ([value, chinese, english]) =>
        `<option value="${value}">${tr(chinese, english)}</option>`,
    )
    .join("");
}

function ultraNoteOnboardingMarkup(): string {
  const courses = ultraNoteCourses.length
    ? ultraNoteCourses
        .map(
          (course) => `
          <button class="list-item ultranote-course-row" type="button" data-native-action="ultranote-bind-course" data-course-id="${escapeHtml(course.courseId)}">
            <strong>${escapeHtml(course.code ? `${course.code} · ${course.title}` : course.title)}</strong>
            <span>${course.mode === "limited" ? tr("受限模式", "Limited Mode") : tr("大纲已启用", "Syllabus active")} · ${tr("更新于", "updated")} ${escapeHtml(course.updatedAt)}</span>
          </button>`,
        )
        .join("")
    : `<p class="ultranote-empty">${tr("还没有课程，请为当前线程创建一个。", "No courses yet. Create one for this thread.")}</p>`;
  return `
    <div class="ultranote-onboarding">
      <section class="provider-card">
        <div class="pane-head" style="padding-inline:0"><div><strong>${tr("选择课程", "Select a course")}</strong><span class="settings-help">${tr("课程记忆只在绑定到同一课程的课堂线程之间共享。", "Course Memory is shared only by lecture threads bound to the same course.")}</span></div></div>
        <div class="compact-list">${courses}</div>
      </section>
      <section class="provider-card">
        <div class="pane-head" style="padding-inline:0"><div><strong>${tr("创建课程", "Create course")}</strong><span class="settings-help">${tr("建议导入课程大纲；也可用受限模式开始，未知规则会保持未确认。", "A syllabus is recommended. Limited Mode can begin without one and keeps unknown policies unresolved.")}</span></div></div>
        <div class="settings-grid">
          <div class="field"><label for="ultraCourseTitle">${tr("课程名称", "Course title")}</label><input class="input" id="ultraCourseTitle" placeholder="${tr("分布式系统", "Distributed Systems")}"></div>
          <div class="field"><label for="ultraCourseCode">${tr("课程代码", "Course code")}</label><input class="input" id="ultraCourseCode" placeholder="CS-501"></div>
          <div class="field wide"><label><input type="checkbox" id="ultraLimitedMode"> ${tr("无大纲，以受限模式开始", "Start in Limited Mode without a syllabus")}</label></div>
        </div>
        <button class="primary-btn" type="button" data-native-action="ultranote-create-course">${tr("创建并绑定当前线程", "Create and bind this thread")}</button>
      </section>
    </div>`;
}

function ultraNoteWorkspaceMarkup(workspace: UltraNoteWorkspace): string {
  const course = workspace.course;
  if (!course) return ultraNoteOnboardingMarkup();
  const syllabus = workspace.syllabus;
  const memory = workspace.memory;
  const syllabusPanel = syllabus
    ? `
      <section class="provider-card ultranote-syllabus-summary">
        <header><div><strong>Syllabus revision ${syllabus.revision}</strong><small>${syllabus.structure.learningObjectives.length} objectives · ${syllabus.structure.topicSchedule.length} schedule entries · ${syllabus.ambiguities.length} unresolved</small></div><span class="status ${syllabus.confirmed ? "complete" : "waiting"}">${syllabus.confirmed ? "CONFIRMED" : "REVIEW"}</span></header>
        ${
          syllabus.ambiguities.length
            ? `<div class="ultranote-alert"><strong>Do not guess</strong>${syllabus.ambiguities.map((item) => `<span>${escapeHtml(item)}</span>`).join("")}</div>`
            : ""
        }
        <div class="ultranote-plan"><div><strong>15–30 min pre-study</strong>${syllabus.prestudyPlan.map((item) => `<span>${escapeHtml(item)}</span>`).join("")}</div><div><strong>First phase</strong>${syllabus.firstPhasePlan.map((item) => `<span>${escapeHtml(item)}</span>`).join("")}</div></div>
      </section>`
    : `
      <section class="provider-card ultranote-syllabus-summary">
        <header><div><strong>Syllabus requested</strong><small>Paste the syllabus below, or continue in Limited Mode. Exams, deadlines, and AI policy remain unknown until confirmed.</small></div><span class="status waiting">LIMITED</span></header>
      </section>`;
  const noteRows = workspace.latestNotes.length
    ? workspace.latestNotes
        .slice(0, 8)
        .map(
          (note) => `
            <article class="ultranote-note-row">
              <div><strong>${escapeHtml(note.title)}</strong><span>Revision ${note.revision} · ${note.sections.length} uniform sections · ${note.sourceMap.length} citation anchors</span></div>
              <div class="native-worker-tags">${[...new Set(note.sections.map((section) => section.provenance))].map((item) => `<span class="native-worker-tag">${escapeHtml(item)}</span>`).join("")}</div>
            </article>`,
        )
        .join("")
    : '<p class="ultranote-empty">No lecture notes yet. Import the first lecture source below.</p>';
  const policy = ultraNotePolicy
    ? `<div class="ultranote-policy-result ${ultraNotePolicy.clarificationRequired ? "waiting" : "complete"}"><strong>${ultraNotePolicy.clarificationRequired ? "Clarification required" : "Assistance boundary ready"}</strong><span>Allowed modes: ${escapeHtml(ultraNotePolicy.allowedModes.join(", "))}</span><span>Direct submittable answer: never</span>${ultraNotePolicy.rationale.map((item) => `<span>${escapeHtml(item)}</span>`).join("")}</div>`
    : "";
  return `
    <div class="ultranote-grid">
      <div class="ultranote-main">
        ${syllabusPanel}
        <section class="provider-card">
          <div class="pane-head" style="padding-inline:0"><div><strong>Lecture ingestion</strong><span class="settings-help">Each independent lecture thread produces uniform notes with provenance and a source map.</span></div></div>
          <div class="settings-grid">
            <div class="field"><label for="ultraLectureKind">Source kind</label><select class="select" id="ultraLectureKind">${sourceKindOptions()}</select></div>
            <div class="field"><label for="ultraLectureName">Display name</label><input class="input" id="ultraLectureName" placeholder="Week 1 · Processes"></div>
            <div class="field wide"><label for="ultraLectureContent">Extracted or pasted content</label><textarea class="textarea" id="ultraLectureContent" placeholder="Paste slides, notes, transcript, reading, board-photo OCR, lab material, code, or dataset context…"></textarea></div>
          </div>
          <button class="primary-btn" type="button" data-native-action="ultranote-ingest-lecture">Generate cited notes</button>
        </section>
        <section class="provider-card">
          <div class="pane-head" style="padding-inline:0"><div><strong>Lecture notes</strong><span class="settings-help">From class, Agent explanation, Inference, External source, and Unresolved remain visibly distinct.</span></div></div>
          <div class="ultranote-notes">${noteRows}</div>
        </section>
      </div>
      <aside class="ultranote-rail">
        <section class="provider-card">
          <strong>Course Memory</strong>
          <div class="ultranote-metrics">
            <span><b>${memory?.objectives.length ?? 0}</b> objectives</span>
            <span><b>${memory?.concepts.length ?? 0}</b> concepts</span>
            <span><b>${memory?.assignments.length ?? 0}</b> assignments</span>
            <span><b>${memory?.reviewItems.length ?? 0}</b> review items</span>
          </div>
          <small>Scoped to ${escapeHtml(course.courseId)}. Other courses, raw chats, and unrelated tool logs are excluded.</small>
          <div class="field" style="margin-top:12px"><label for="ultraSearchQuery">Course retrieval</label><div class="row"><input class="input" id="ultraSearchQuery" placeholder="Search imported course sources" style="flex:1"><button class="ghost-btn" type="button" data-native-action="ultranote-search">Search</button></div></div>
          <div class="ultranote-search-results">${ultraNoteSearchHits.map((hit) => `<div class="ultranote-search-hit"><strong>${escapeHtml(hit.sourceId)}</strong><span>${escapeHtml(hit.excerpt)}</span></div>`).join("")}</div>
        </section>
        <section class="provider-card">
          <strong>Syllabus import / revision</strong>
          <div class="field" style="margin-top:10px"><label for="ultraSyllabusName">Source name</label><input class="input" id="ultraSyllabusName" value="Course syllabus"></div>
          <div class="field"><label for="ultraSyllabusContent">Syllabus text</label><textarea class="textarea" id="ultraSyllabusContent" placeholder="Paste the current syllabus revision…"></textarea></div>
          <button class="ghost-btn" type="button" data-native-action="ultranote-ingest-syllabus">${syllabus ? "Add syllabus revision" : "Extract syllabus"}</button>
        </section>
        <section class="provider-card">
          <strong>Homework integrity guard</strong>
          <div class="field" style="margin-top:10px"><label for="ultraHomeworkKind">Classification</label><select class="select" id="ultraHomeworkKind">${(["practice", "ungraded", "graded", "exam", "unknown"] as HomeworkKind[]).map((kind) => `<option value="${kind}">${kind}</option>`).join("")}</select></div>
          <button class="ghost-btn" type="button" data-native-action="ultranote-evaluate-homework">Check assistance boundary</button>
          ${policy}
        </section>
        <button class="primary-btn" type="button" data-native-action="ultranote-export">Export course Markdown</button>
      </aside>
    </div>`;
}

function renderUltraNote(): void {
  const view = document.querySelector<HTMLElement>("#view");
  if (!view) return;
  const workspace = ultraNoteWorkspace;
  view.dataset.nativeUltranote = "true";
  view.classList.remove("orch-active");
  view.scrollTop = 0;
  view.innerHTML = `
    <section class="ultranote-shell">
      <header class="ultranote-header">
        <div><span class="ultranote-kicker">${tr("命令触发的学习工作流", "COMMAND-ACTIVATED LEARNING WORKFLOW")}</span><h2>/ultranote${workspace?.course ? ` · ${escapeHtml(workspace.course.title)}` : ""}</h2><p>${tr("基于课程来源的预习、引用笔记、透明复习与符合规则的作业辅助。", "Course-grounded pre-study, cited lecture notes, transparent review, and policy-safe homework support.")}</p></div>
        <div class="row"><span class="status ${isTauri() ? "complete" : "waiting"}">${isTauri() ? "NATIVE SQLITE" : "PREVIEW"}</span><button class="ghost-btn" type="button" data-native-action="ultranote-refresh">Refresh</button></div>
      </header>
      <div class="settings-status" id="ultraNoteStatus" role="status"></div>
      ${workspace ? ultraNoteWorkspaceMarkup(workspace) : '<div class="provider-card">Loading UltraNote workspace…</div>'}
    </section>`;
}

async function openUltraNoteWorkspace(): Promise<void> {
  const threadId = currentThreadId();
  document
    .querySelector<HTMLElement>(".workspace")
    ?.classList.add("special-view");
  document.querySelector<HTMLElement>("#tabs")?.classList.add("hidden");
  document.querySelector<HTMLElement>("#composer")?.classList.add("hidden");
  document
    .querySelectorAll<HTMLElement>(".side-link")
    .forEach((item) =>
      item.classList.toggle(
        "active",
        item.dataset.special === "ultranote",
      ),
    );
  const title = document.querySelector<HTMLElement>("#headerTitle");
  if (title) title.textContent = "LunaScope / Command · /ultranote";
  history.replaceState(
    null,
    "",
    `#/project/lunascope/thread/${encodeURIComponent(threadId)}/ultranote`,
  );
  renderUltraNote();
  if (!isTauri()) {
    ultraNoteCourses = [];
    ultraNoteWorkspace = {
      threadId,
      binding: null,
      course: null,
      syllabus: null,
      latestNotes: [],
      memory: null,
      requestedAction: "create_or_select_course",
    };
    renderUltraNote();
    return;
  }
  try {
    [ultraNoteWorkspace, ultraNoteCourses] = await Promise.all([
      invoke<UltraNoteWorkspace>("activate_ultranote", { threadId }),
      invoke<Course[]>("list_courses"),
    ]);
    renderUltraNote();
  } catch (error) {
    ultraNoteStatus(`UltraNote failed to load: ${errorMessage(error)}`, "error");
  }
}

async function handleUltraNoteAction(button: HTMLButtonElement): Promise<boolean> {
  const action = button.dataset.nativeAction ?? "";
  if (!action.startsWith("ultranote-")) return false;
  if (action === "ultranote-refresh") {
    await openUltraNoteWorkspace();
    return true;
  }
  if (!isTauri()) {
    ultraNoteStatus("Browser preview is read-only. Run the Tauri app for durable course actions.", "waiting");
    return true;
  }
  try {
    if (action === "ultranote-create-course") {
      const title =
        document.querySelector<HTMLInputElement>("#ultraCourseTitle")?.value ??
        "";
      const code =
        document.querySelector<HTMLInputElement>("#ultraCourseCode")?.value ??
        "";
      const limitedMode =
        document.querySelector<HTMLInputElement>("#ultraLimitedMode")
          ?.checked ?? false;
      ultraNoteWorkspace = await invoke<UltraNoteWorkspace>("create_course", {
        request: {
          title,
          code: code || null,
          limitedMode,
          threadId: currentThreadId(),
        },
      });
      ultraNoteCourses = await invoke<Course[]>("list_courses");
      renderUltraNote();
      ultraNoteStatus("Course created and this lecture thread is bound.", "success");
      return true;
    }
    if (action === "ultranote-bind-course") {
      ultraNoteWorkspace = await invoke<UltraNoteWorkspace>(
        "bind_course_thread",
        {
          request: {
            courseId: button.dataset.courseId ?? "",
            threadId: currentThreadId(),
          },
        },
      );
      renderUltraNote();
      ultraNoteStatus("Course selected. Course Memory is now available to this thread.", "success");
      return true;
    }
    const courseId = ultraNoteWorkspace?.course?.courseId;
    if (!courseId) throw new Error("Create or select a course first.");
    if (action === "ultranote-ingest-syllabus") {
      const displayName =
        document.querySelector<HTMLInputElement>("#ultraSyllabusName")?.value ??
        "";
      const content =
        document.querySelector<HTMLTextAreaElement>("#ultraSyllabusContent")
          ?.value ?? "";
      ultraNoteStatus("Extracting syllabus structure without guessing missing policy…");
      await invoke("ingest_syllabus", {
        request: { courseId, displayName, content },
      });
      ultraNoteWorkspace = await invoke<UltraNoteWorkspace>(
        "activate_ultranote",
        { threadId: currentThreadId() },
      );
      renderUltraNote();
      ultraNoteStatus("Syllabus revision saved. Ambiguities remain explicit until confirmed.", "success");
      return true;
    }
    if (action === "ultranote-search") {
      const query =
        document.querySelector<HTMLInputElement>("#ultraSearchQuery")?.value ??
        "";
      ultraNoteSearchHits = await invoke<CourseSearchHit[]>(
        "search_ultranote",
        { courseId, query },
      );
      renderUltraNote();
      ultraNoteStatus(
        `${ultraNoteSearchHits.length} course-scoped source matches.`,
        "success",
      );
      return true;
    }
    if (action === "ultranote-ingest-lecture") {
      const kind =
        (document.querySelector<HTMLSelectElement>("#ultraLectureKind")
          ?.value as CourseSourceKind) ?? "lecture_notes";
      const displayName =
        document.querySelector<HTMLInputElement>("#ultraLectureName")?.value ??
        "";
      const content =
        document.querySelector<HTMLTextAreaElement>("#ultraLectureContent")
          ?.value ?? "";
      ultraNoteStatus("Generating uniform notes and citation anchors…");
      await invoke("ingest_lecture", {
        request: {
          courseId,
          threadId: currentThreadId(),
          kind,
          displayName,
          content,
        },
      });
      ultraNoteWorkspace = await invoke<UltraNoteWorkspace>(
        "activate_ultranote",
        { threadId: currentThreadId() },
      );
      renderUltraNote();
      ultraNoteStatus("Lecture notes saved with provenance and Source Map.", "success");
      return true;
    }
    if (action === "ultranote-evaluate-homework") {
      const kind =
        (document.querySelector<HTMLSelectElement>("#ultraHomeworkKind")
          ?.value as HomeworkKind) ?? "unknown";
      ultraNotePolicy = await invoke<HomeworkPolicyDecision>(
        "evaluate_homework",
        { request: { courseId, kind } },
      );
      renderUltraNote();
      ultraNoteStatus("Homework assistance boundary evaluated.", "success");
      return true;
    }
    if (action === "ultranote-export") {
      const path = await invoke<string>("export_ultranote", {
        threadId: currentThreadId(),
      });
      ultraNoteStatus(`Course export written to ${path}`, "success");
      return true;
    }
  } catch (error) {
    ultraNoteStatus(errorMessage(error), "error");
  }
  return true;
}

function workerRoleOptions(selected = ""): string {
  return workerRoleCatalog
    .map(
      ([value, label]) =>
        `<option value="${value}" ${value === selected ? "selected" : ""}>${label}</option>`,
    )
    .join("");
}

function tagsForRole(role: string): string[] {
  return (
    workerRoleCatalog.find(([value]) => value === role)?.[2] ?? role
  )
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

function workerTagMarkup(tags: string[]): string {
  return `<span class="native-worker-tags">${tags
    .slice(0, 4)
    .map((tag) => `<span class="native-worker-tag">${escapeHtml(tag)}</span>`)
    .join("")}</span>`;
}

function verifierOutputSchema(): WorkerSpec["outputSchema"] {
  return {
    mediaType: "application/json",
    schema: {
      type: "object",
      required: ["summary", "evidence", "verification"],
      properties: {
        summary: { type: "string" },
        evidence: { type: "array", items: { type: "string" } },
        verification: {
          type: "object",
          required: [
            "status",
            "summary",
            "evidence",
            "remainingRisks",
            "findings",
          ],
          properties: {
            status: {
              enum: [
                "verified",
                "partially_verified",
                "unverified",
                "unable_to_verify",
                "failed_verification",
              ],
            },
            summary: { type: "string" },
            evidence: { type: "array", items: { type: "string" } },
            remainingRisks: { type: "array", items: { type: "string" } },
            findings: {
              type: "array",
              items: {
                type: "object",
                required: [
                  "severity",
                  "title",
                  "description",
                  "affectedPaths",
                  "repairHint",
                ],
                properties: {
                  severity: { enum: ["fatal", "major", "minor"] },
                  title: { type: "string" },
                  description: { type: "string" },
                  affectedPaths: {
                    type: "array",
                    items: { type: "string" },
                  },
                  repairHint: { type: "string" },
                },
                additionalProperties: false,
              },
            },
          },
        },
      },
      additionalProperties: true,
    },
  };
}

function evidenceOutputSchema(): WorkerSpec["outputSchema"] {
  return {
    mediaType: "application/json",
    schema: {
      type: "object",
      required: ["summary", "evidence"],
      properties: {
        summary: { type: "string" },
        evidence: { type: "array", items: { type: "string" } },
      },
      additionalProperties: true,
    },
  };
}

function roleAssignment(role: string, objective: string) {
  const assignments: Record<
    string,
    { task: string; expectedOutput: string; readOnly: boolean }
  > = {
    planner: {
      task: "Decompose the objective, constraints, dependencies, and acceptance evidence.",
      expectedOutput: "A structured execution plan and risk map.",
      readOnly: true,
    },
    builder: {
      task: "Implement the assigned change within an explicit write scope.",
      expectedOutput: "A reviewable patch artifact and implementation summary.",
      readOnly: false,
    },
    frontend: {
      task: "Implement and verify the assigned frontend or interaction slice.",
      expectedOutput: "A scoped frontend patch with responsive verification evidence.",
      readOnly: false,
    },
    backend: {
      task: "Implement and verify the assigned backend or runtime slice.",
      expectedOutput: "A scoped backend patch with test evidence.",
      readOnly: false,
    },
    researcher: {
      task: "Research the objective using supplied sources and isolate evidence, gaps, and risks.",
      expectedOutput: "A source-grounded research artifact.",
      readOnly: true,
    },
    reviewer: {
      task: "Review upstream evidence independently and challenge unsupported claims.",
      expectedOutput: "An independent read-only review artifact.",
      readOnly: true,
    },
    verifier: {
      task: "Verify completion criteria independently from upstream implementation.",
      expectedOutput: "A typed VerificationRecord with evidence and remaining risks.",
      readOnly: true,
    },
    game_designer: {
      task: "Design or implement the assigned game-development slice without modifying binary assets.",
      expectedOutput: "A scoped game-design or code artifact with engine-aware evidence.",
      readOnly: false,
    },
    academic_writer: {
      task: "Develop the assigned academic-writing section with explicit source and claim boundaries.",
      expectedOutput: "A source-mapped academic writing artifact.",
      readOnly: true,
    },
    documentation: {
      task: "Write and verify the assigned documentation slice.",
      expectedOutput: "A scoped documentation patch with reviewed claims.",
      readOnly: false,
    },
  };
  const assignment = assignments[role] ?? {
    task: `Complete the user-labelled ${role} workstream.`,
    expectedOutput: "A structured artifact matching the declared role.",
    readOnly: true,
  };
  return {
    ...assignment,
    prompt: `Role: ${role}\nObjective: ${objective}\nTask: ${assignment.task}\nReturn only schema-valid JSON. Respect every declared permission, dependency, label, and hard constraint.`,
  };
}

function projectNativeState(snapshot: RuntimeSnapshot): void {
  document.documentElement.dataset.runtimeAuthority = "native";
  document.documentElement.dataset.runtimeSequence = String(snapshot.sequence);
  document.documentElement.dataset.runtimeState = snapshot.runState;

  const status = document.querySelector<HTMLElement>("#statusText");
  if (status) {
    status.textContent =
      `NATIVE · ${snapshot.runState.toUpperCase()} · SEQ ${snapshot.sequence}`;
  }

  window.dispatchEvent(
    new CustomEvent("lunascope:native-snapshot", { detail: snapshot }),
  );
}

function applyMessage(message: RuntimeMessage): void {
  if (message.kind === "snapshot") {
    projectNativeState(message.data);
    return;
  }

  const current = Number(
    document.documentElement.dataset.runtimeSequence ?? "0",
  );
  if (message.data.fromSequence !== current) {
    console.error("Native runtime sequence gap", {
      expected: current,
      received: message.data.fromSequence,
    });
    void subscribe(current);
    return;
  }

  document.documentElement.dataset.runtimeSequence = String(
    message.data.toSequence,
  );
  window.dispatchEvent(
    new CustomEvent("lunascope:native-delta", { detail: message.data }),
  );
}

async function subscribe(afterSequence: number): Promise<void> {
  const channel = new Channel<RuntimeMessage>();
  channel.onmessage = applyMessage;
  await invoke("subscribe_run", {
    runId: bootstrapRunId,
    afterSequence,
    onMessage: channel,
  });
}

function generalSettingsMarkup(): string {
  return `
    <div class="pane-head" style="padding-inline:0"><div><strong>${tr("通用", "General")}</strong><span class="settings-help">${tr("仅保留影响整个应用的必要设置。", "Only essential application-wide preferences are kept here.")}</span></div></div>
    <div class="setting-row">
      <div><strong>${tr("界面语言", "Interface language")}</strong><span>${tr("切换简体中文或英文。", "Switch between Simplified Chinese and English.")}</span></div>
      <select class="select" id="uiLanguage" style="max-width:220px"><option value="chinese" ${userPreferences.language === "chinese" ? "selected" : ""}>简体中文</option><option value="english" ${userPreferences.language === "english" ? "selected" : ""}>English</option></select>
    </div>
    <div class="setting-row">
      <div><strong>${tr("模型回复语言", "Model reply language")}</strong><span>${tr("独立于界面语言，约束 Orchestrator、Workers 与最终回复。", "Independent from the UI language; constrains the Orchestrator, Workers, and final response.")}</span></div>
      <select class="select" id="modelReplyLanguage" style="max-width:220px">
        <option value="follow_ui" ${userPreferences.modelReplyLanguage === "follow_ui" ? "selected" : ""}>${tr("跟随界面", "Follow interface")}</option>
        <option value="chinese" ${userPreferences.modelReplyLanguage === "chinese" ? "selected" : ""}>简体中文</option>
        <option value="english" ${userPreferences.modelReplyLanguage === "english" ? "selected" : ""}>English</option>
      </select>
    </div>
    <div class="row" style="margin-top:14px"><button class="primary-btn" type="button" data-native-action="save-general-preferences">${tr("保存语言设置", "Save language settings")}</button></div>
    <div class="settings-status" id="generalPreferencesStatus" role="status"></div>`;
}

function projectSettingsMarkup(): string {
  return `
    <div class="pane-head" style="padding-inline:0"><div><strong>${tr("项目与文件夹", "Projects & folders")}</strong><span class="settings-help">${tr("每个项目可包含多个文件夹，但必须指定一个 workspace。", "Each project can contain multiple folders, with exactly one workspace.")}</span></div><button class="primary-btn" type="button" data-native-action="project-open-manager">${tr("打开项目管理", "Open project manager")}</button></div>
    <div class="provider-card"><strong>${projects.length} ${tr("个项目", "projects")}</strong><small>${tr("新增、移除文件夹或切换 workspace 都在项目管理中完成。", "Add or remove folders and change the workspace in Project management.")}</small></div>`;
}

function ultraNoteSettingsMarkup(): string {
  return `
    <div class="pane-head" style="padding-inline:0"><div><strong>UltraNote</strong><span class="settings-help">${tr("UltraNote 由 /ultranote 触发；这里仅保存你的笔记规范。", "UltraNote is triggered by /ultranote; this page only stores your note specification.")}</span></div></div>
    <div class="field"><label for="ultraNoteSpec">${tr("自定义笔记规范", "Custom note specification")}</label><textarea class="textarea" id="ultraNoteSpec" style="min-height:220px">${escapeHtml(userPreferences.ultranoteNoteSpec)}</textarea><span class="settings-help">${tr("规范会附加到笔记生成，但不会削弱来源标注、引用锚点、课程隔离或学术诚信边界。", "The specification guides note generation without weakening provenance, citation anchors, course isolation, or academic-integrity boundaries.")}</span></div>
    <button class="primary-btn" type="button" data-native-action="save-ultranote-preferences">${tr("保存笔记规范", "Save note specification")}</button>
    <div class="settings-status" id="ultraNotePreferencesStatus" role="status"></div>`;
}

function providerOptions(selected: ProviderType): string {
  const options: Array<[ProviderType, string]> = [
    ["open_ai", "OpenAI"],
    ["anthropic", "Anthropic"],
    ["deep_seek", "DeepSeek"],
    ["generic_open_ai_compatible", "OpenAI-compatible"],
    ["generic_anthropic_compatible", "Anthropic-compatible"],
  ];
  return options
    .map(
      ([value, label]) =>
        `<option value="${value}" ${value === selected ? "selected" : ""}>${label}</option>`,
    )
    .join("");
}

function protocolOptions(selected: ProviderProtocol): string {
  const options: Array<[ProviderProtocol, string]> = [
    ["open_ai_responses", "OpenAI Responses"],
    ["open_ai_chat_completions", "OpenAI Chat Completions"],
    ["anthropic_messages", "Anthropic Messages"],
  ];
  return options
    .map(
      ([value, label]) =>
        `<option value="${value}" ${value === selected ? "selected" : ""}>${label}</option>`,
    )
    .join("");
}

const workerRoles: Array<[ModelRole, string]> = [
  ["general_worker", "Default Worker"],
  ["programming", "Programming"],
  ["research", "Research"],
  ["writing", "Writing"],
  ["frontend", "Frontend"],
  ["game_development", "Game Development"],
  ["reviewer", "Reviewer"],
  ["verifier", "Verifier"],
  ["fast_cheap", "Fast / Cheap"],
  ["vision", "Vision"],
];

function assignmentForRole(role: ModelRole): ModelAssignment | undefined {
  return modelSelectionSettings.workerPool.find(
    (assignment) => assignment.role === role,
  );
}

function workerPoolMarkup(): string {
  return workerRoles
    .map(([role, label]) => {
      const assignment = assignmentForRole(role);
      return `<div class="model-pool-row" data-worker-model-row="${role}">
        <strong>${label}</strong>
        <input class="input" data-worker-provider value="${escapeHtml(assignment?.providerConfigId ?? "")}" placeholder="provider config ID" aria-label="${label} provider">
        <input class="input" data-worker-model value="${escapeHtml(assignment?.modelId ?? "")}" placeholder="model ID" aria-label="${label} model">
        <select class="select" data-worker-effort aria-label="${label} reasoning effort"><option value="low" ${assignment?.reasoningEffort === "low" ? "selected" : ""}>Low</option><option value="medium" ${!assignment || assignment.reasoningEffort === "medium" ? "selected" : ""}>Medium</option><option value="high" ${assignment?.reasoningEffort === "high" ? "selected" : ""}>High</option></select>
        <label><input type="checkbox" data-worker-locked ${assignment?.locked ? "checked" : ""}> Lock</label>
      </div>`;
    })
    .join("");
}

function providerSettingsMarkup(): string {
  return `<div class="pane-head" style="padding-inline:0"><div><strong>Models &amp; Providers</strong><span class="settings-help">多个 Provider 可同时保存和启用；相同 ID 才会更新已有配置。API Key 仅写入 Windows Credential Manager。</span></div><div class="row"><button class="primary-btn" type="button" data-native-action="new-provider">新建 Provider</button><button class="ghost-btn" type="button" data-native-action="refresh-providers">刷新</button></div></div>
    <form id="providerConfigForm" autocomplete="off">
      <div class="settings-grid">
        <div class="field"><label for="providerId">配置 ID</label><input class="input" id="providerId" value="openai-primary" required pattern="[A-Za-z0-9._-]+"></div>
        <div class="field"><label for="providerDisplayName">显示名称</label><input class="input" id="providerDisplayName" value="OpenAI Primary" required></div>
        <div class="field"><label for="providerType">提供商</label><select class="select" id="providerType">${providerOptions("open_ai")}</select></div>
        <div class="field"><label for="providerProtocol">协议</label><select class="select" id="providerProtocol">${protocolOptions("open_ai_responses")}</select></div>
        <div class="field wide"><label for="providerBaseUrl">Base URL</label><input class="input" id="providerBaseUrl" type="url" value="https://api.openai.com" required></div>
        <div class="field"><label for="providerCredentialReference">凭据引用 ID</label><input class="input" id="providerCredentialReference" value="credential-openai-primary" required pattern="[A-Za-z0-9._-]+"></div>
        <div class="field"><label for="providerCredential">API Key（留空则保留现有凭据）</label><input class="input" id="providerCredential" type="password" autocomplete="new-password" spellcheck="false"></div>
        <div class="field wide"><label for="providerHeaders">自定义 Headers（JSON 数组；敏感 Header 必须使用 credential_reference）</label><textarea class="textarea" id="providerHeaders" spellcheck="false" style="min-height:74px">[]</textarea></div>
        <div class="field"><label for="providerTestModel">默认模型 / 连接测试模型</label><input class="input" id="providerTestModel" value="gpt-5-mini" required></div>
        <div class="field"><label for="providerContextWindow">Context window tokens（可选）</label><input class="input" id="providerContextWindow" type="number" min="1" value="128000"></div>
        <div class="field wide"><label>Provider capabilities</label><div class="row"><label><input id="providerSupportsTools" type="checkbox" checked> Tools</label><label><input id="providerSupportsVision" type="checkbox"> Vision</label><label><input id="providerSupportsStructured" type="checkbox" checked> Structured output</label><label><input id="providerEnabled" type="checkbox" checked> Enabled</label></div></div>
      </div>
      <div class="row"><button class="primary-btn" type="submit">保存配置</button><button class="ghost-btn" type="button" data-native-action="test-provider">测试连接</button></div>
      <div class="settings-status" id="providerActionStatus" role="status"></div>
    </form>
    <section style="margin-top:20px"><h3>已保存配置</h3><div id="providerConfigList"><span class="settings-help">正在读取本机配置…</span></div></section>
    <section style="margin-top:24px"><div class="pane-head" style="padding-inline:0"><div><strong>Orchestration Model</strong><span class="settings-help">编排、计划、Worker Graph、失败处理与最终汇总使用独立配置。</span></div></div>
      <div class="settings-grid">
        <div class="field"><label for="orchestrationProvider">Provider config ID</label><input class="input" id="orchestrationProvider" value="${escapeHtml(modelSelectionSettings.orchestration.providerConfigId)}"></div>
        <div class="field"><label for="orchestrationModel">Model ID</label><input class="input" id="orchestrationModel" value="${escapeHtml(modelSelectionSettings.orchestration.modelId)}"></div>
        <div class="field"><label for="orchestrationEffort">Reasoning effort</label><select class="select" id="orchestrationEffort"><option value="low">Low</option><option value="medium">Medium</option><option value="high" selected>High</option></select></div>
        <div class="field"><label for="orchestrationContext">Maximum context tokens</label><input class="input" id="orchestrationContext" type="number" min="1"></div>
        <div class="field"><label for="orchestrationBudget">Maximum budget (microusd)</label><input class="input" id="orchestrationBudget" type="number" min="1"></div>
        <label class="field" style="align-content:end"><span><input id="orchestrationLocked" type="checkbox"> 用户锁定</span></label>
        <div class="field"><label for="orchestrationFallbackProvider">Fallback provider config ID</label><input class="input" id="orchestrationFallbackProvider"></div>
        <div class="field"><label for="orchestrationFallbackModel">Fallback model ID</label><input class="input" id="orchestrationFallbackModel"></div>
      </div>
      <div class="pane-head" style="padding-inline:0"><div><strong>Worker Model Pool</strong><span class="settings-help">留空的角色由 Default Worker 或路由策略选择；锁定项不会被 Orchestrator 覆盖。</span></div></div>
      <div class="model-pool" id="workerModelPool">${workerPoolMarkup()}</div>
      <button class="primary-btn" type="button" data-native-action="save-model-settings">保存编排与 Worker 模型</button>
      <div class="settings-status" id="modelSettingsStatus" role="status"></div>
    </section>
    <section style="margin-top:24px"><div class="pane-head" style="padding-inline:0"><div><strong>Structured Preferences</strong><span class="settings-help">0–100 优先级、Provider 允许列表和单次运行成本上限。</span></div></div>
      <div class="settings-grid">
        <div class="field"><label for="priorityQuality">Quality</label><input class="input" id="priorityQuality" type="number" min="0" max="100"></div>
        <div class="field"><label for="priorityCost">Cost</label><input class="input" id="priorityCost" type="number" min="0" max="100"></div>
        <div class="field"><label for="prioritySpeed">Speed</label><input class="input" id="prioritySpeed" type="number" min="0" max="100"></div>
        <div class="field"><label for="priorityPrivacy">Privacy</label><input class="input" id="priorityPrivacy" type="number" min="0" max="100"></div>
        <div class="field"><label for="allowedProviders">Allowed provider config IDs（逗号分隔）</label><input class="input" id="allowedProviders"></div>
        <div class="field"><label for="disallowedProviders">Disallowed provider config IDs（逗号分隔）</label><input class="input" id="disallowedProviders"></div>
        <div class="field"><label for="maximumRunCost">Maximum cost per run (microusd)</label><input class="input" id="maximumRunCost" type="number" min="1"></div>
        <div class="field"><label>Behavior</label><div class="row"><label><input id="preferLocal" type="checkbox"> Prefer local</label><label><input id="fallbackAllowed" type="checkbox"> Allow fallback</label><label><input id="askCostEscalation" type="checkbox"> Ask on cost escalation</label></div></div>
      </div>
      <button class="ghost-btn" type="button" data-native-action="save-structured-routing">保存结构化偏好</button>
    </section>
    <section style="margin-top:24px"><div class="pane-head" style="padding-inline:0"><div><strong>自然语言路由偏好</strong><span class="settings-help">解析只生成草案；请检查命中规则与警告，再明确确认保存。</span></div></div>
      <div class="field"><label for="routingPreference">偏好说明</label><textarea class="textarea" id="routingPreference" style="min-height:90px">编程任务优先 OpenAI，写作优先 Claude，日常任务优先 DeepSeek；如果提高成本先问我。</textarea></div>
      <div class="row"><button class="ghost-btn" type="button" data-native-action="parse-routing">解析为草案</button><button class="primary-btn" type="button" data-native-action="confirm-routing" disabled>确认并保存</button></div>
      <div class="settings-status" id="routingActionStatus" role="status"></div>
      <div id="routingReview"></div>
    </section>`;
}

function setSettingsStatus(
  id: string,
  message: string,
  kind: "idle" | "error" | "success" = "idle",
): void {
  const element = document.querySelector<HTMLElement>(`#${id}`);
  if (!element) return;
  element.textContent = message;
  element.className = `settings-status ${kind === "idle" ? "" : kind}`;
}

function renderProviderList(): void {
  const list = document.querySelector<HTMLElement>("#providerConfigList");
  if (!list) return;
  if (providerConfigs.length === 0) {
    list.innerHTML =
      '<div class="provider-card"><span class="settings-help">尚未保存 Provider 配置。</span></div>';
    return;
  }
  list.innerHTML = providerConfigs
    .map(
      (config, index) =>
        `<article class="provider-card"><header><div><strong>${escapeHtml(config.displayName)}</strong><small>${escapeHtml(config.providerType)} · ${escapeHtml(config.protocol)} · ${escapeHtml(config.baseUrl)}</small><small>Default model · ${escapeHtml(config.defaultModelId || "not configured")} · Context ${config.contextWindowTokens ?? "unknown"}</small><small>Capabilities · ${[config.supportsTools && "tools", config.supportsVision && "vision", config.supportsStructuredOutput && "structured"].filter(Boolean).join(", ") || "text only"} · Credential reference ${escapeHtml(config.credentialReferenceId)}</small></div><span class="status ${config.enabled ? "complete" : "waiting"}">${config.enabled ? "Enabled" : "Disabled"}</span></header><div class="row" style="margin-top:10px"><button class="ghost-btn" type="button" data-native-action="edit-provider" data-provider-index="${index}">载入编辑</button><button class="text-btn" type="button" data-native-action="delete-credential" data-provider-index="${index}">删除凭据</button></div></article>`,
    )
    .join("");
}

function renderRoutingPolicy(): void {
  const review = document.querySelector<HTMLElement>("#routingReview");
  if (!review) return;
  if (!pendingRoutingDraft) {
    const preferences =
      routingPolicy.rolePreferences.length === 0
        ? "尚无角色偏好"
        : routingPolicy.rolePreferences
            .map(
              (preference) =>
                `${preference.role} → ${preference.preferredProviders.join(", ")}${preference.hardConstraint ? " (hard)" : ""}`,
            )
            .join("\n");
    review.innerHTML = `<div class="routing-review">当前已保存策略\n${escapeHtml(preferences)}\nFallback: ${routingPolicy.fallbackAllowed ? "allowed" : "disabled"} · Cost escalation approval: ${routingPolicy.askBeforeCostEscalation ? "required" : "not required"}</div>`;
    return;
  }
  const rules = pendingRoutingDraft.matchedRules.length
    ? pendingRoutingDraft.matchedRules.map((rule) => `✓ ${rule}`).join("\n")
    : "未命中可执行规则";
  const warnings = pendingRoutingDraft.warnings.length
    ? `\n\n警告\n${pendingRoutingDraft.warnings.map((warning) => `! ${warning}`).join("\n")}`
    : "";
  const preferences = pendingRoutingDraft.policy.rolePreferences
    .map(
      (preference) =>
        `${preference.role} → ${preference.preferredProviders.join(", ")}${preference.hardConstraint ? " (hard)" : ""}`,
    )
    .join("\n");
  review.innerHTML = `<div class="routing-review">待确认草案\n\n命中规则\n${escapeHtml(rules)}${escapeHtml(warnings)}\n\n角色路由\n${escapeHtml(preferences || "无")}</div>`;
}

function populateProviderForm(config: ProviderConfig): void {
  const values: Record<string, string> = {
    providerId: config.id,
    providerDisplayName: config.displayName,
    providerType: config.providerType,
    providerProtocol: config.protocol,
    providerBaseUrl: config.baseUrl,
    providerCredentialReference: config.credentialReferenceId,
    providerTestModel: config.defaultModelId,
    providerContextWindow: config.contextWindowTokens?.toString() ?? "",
    providerHeaders: JSON.stringify(config.customHeaders, null, 2),
  };
  for (const [id, value] of Object.entries(values)) {
    const element = document.querySelector<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>(
      `#${id}`,
    );
    if (element) element.value = value;
  }
  const enabled = document.querySelector<HTMLInputElement>("#providerEnabled");
  if (enabled) enabled.checked = config.enabled;
  const capabilityChecks: Array<[string, boolean]> = [
    ["providerSupportsTools", config.supportsTools],
    ["providerSupportsVision", config.supportsVision],
    ["providerSupportsStructured", config.supportsStructuredOutput],
  ];
  for (const [id, checked] of capabilityChecks) {
    const input = document.querySelector<HTMLInputElement>(`#${id}`);
    if (input) input.checked = checked;
  }
  const credential =
    document.querySelector<HTMLInputElement>("#providerCredential");
  if (credential) credential.value = "";
  setSettingsStatus(
    "providerActionStatus",
    `已载入 ${config.displayName}；凭据保持为空且不会从系统读取回显。`,
  );
}

function resetProviderForm(): void {
  const defaults: Record<string, string> = {
    providerId: "",
    providerDisplayName: "",
    providerType: "open_ai",
    providerProtocol: "open_ai_responses",
    providerBaseUrl: "https://api.openai.com",
    providerCredentialReference: "",
    providerCredential: "",
    providerTestModel: "",
    providerContextWindow: "",
    providerHeaders: "[]",
  };
  for (const [id, value] of Object.entries(defaults)) {
    const element = document.querySelector<
      HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
    >(`#${id}`);
    if (element) element.value = value;
  }
  for (const id of [
    "providerSupportsTools",
    "providerSupportsStructured",
    "providerEnabled",
  ]) {
    const element = document.querySelector<HTMLInputElement>(`#${id}`);
    if (element) element.checked = true;
  }
  const vision =
    document.querySelector<HTMLInputElement>("#providerSupportsVision");
  if (vision) vision.checked = false;
  setSettingsStatus(
    "providerActionStatus",
    "新建模式：填写唯一配置 ID 后保存，不会覆盖其他 Provider。",
  );
  document.querySelector<HTMLInputElement>("#providerId")?.focus();
}

function readProviderForm(): {
  config: ProviderConfig;
  credential: string | null;
  model: string;
} {
  const value = (id: string): string => {
    const input = document.querySelector<
      HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
    >(`#${id}`);
    if (!input) throw new Error(`missing provider field: ${id}`);
    return input.value.trim();
  };
  const customHeaders = JSON.parse(value("providerHeaders")) as ProviderHeader[];
  if (!Array.isArray(customHeaders)) {
    throw new Error("自定义 Headers 必须是 JSON 数组");
  }
  const credential = value("providerCredential");
  return {
    config: {
      id: value("providerId"),
      providerType: value("providerType") as ProviderType,
      protocol: value("providerProtocol") as ProviderProtocol,
      displayName: value("providerDisplayName"),
      baseUrl: value("providerBaseUrl"),
      credentialReferenceId: value("providerCredentialReference"),
      defaultModelId: value("providerTestModel"),
      customHeaders,
      contextWindowTokens: value("providerContextWindow")
        ? Number(value("providerContextWindow"))
        : null,
      supportsTools:
        document.querySelector<HTMLInputElement>("#providerSupportsTools")
          ?.checked ?? false,
      supportsVision:
        document.querySelector<HTMLInputElement>("#providerSupportsVision")
          ?.checked ?? false,
      supportsStructuredOutput:
        document.querySelector<HTMLInputElement>("#providerSupportsStructured")
          ?.checked ?? false,
      enabled:
        document.querySelector<HTMLInputElement>("#providerEnabled")?.checked ??
        false,
    },
    credential: credential || null,
    model: value("providerTestModel"),
  };
}

function inputValue(id: string): string {
  return (
    document.querySelector<
      HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
    >(`#${id}`)?.value.trim() ?? ""
  );
}

function optionalNumber(id: string): number | null {
  const raw = inputValue(id);
  return raw ? Number(raw) : null;
}

function populateStructuredPolicy(): void {
  const values: Record<string, string> = {
    priorityQuality: routingPolicy.priorities.quality.toString(),
    priorityCost: routingPolicy.priorities.cost.toString(),
    prioritySpeed: routingPolicy.priorities.speed.toString(),
    priorityPrivacy: routingPolicy.priorities.privacy.toString(),
    allowedProviders: routingPolicy.allowedProviderConfigIds.join(", "),
    disallowedProviders: routingPolicy.disallowedProviderConfigIds.join(", "),
    maximumRunCost:
      routingPolicy.maximumCostMicrousdPerRun?.toString() ?? "",
  };
  for (const [id, value] of Object.entries(values)) {
    const input = document.querySelector<HTMLInputElement>(`#${id}`);
    if (input) input.value = value;
  }
  const checks: Array<[string, boolean]> = [
    ["preferLocal", routingPolicy.preferLocal],
    ["fallbackAllowed", routingPolicy.fallbackAllowed],
    ["askCostEscalation", routingPolicy.askBeforeCostEscalation],
  ];
  for (const [id, checked] of checks) {
    const input = document.querySelector<HTMLInputElement>(`#${id}`);
    if (input) input.checked = checked;
  }
}

function readStructuredPolicy(): ModelRoutingPolicy {
  const csv = (id: string): string[] =>
    inputValue(id)
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  const number = (id: string): number => Number(inputValue(id));
  return {
    ...routingPolicy,
    priorities: {
      quality: number("priorityQuality"),
      cost: number("priorityCost"),
      speed: number("prioritySpeed"),
      privacy: number("priorityPrivacy"),
    },
    allowedProviderConfigIds: csv("allowedProviders"),
    disallowedProviderConfigIds: csv("disallowedProviders"),
    maximumCostMicrousdPerRun: optionalNumber("maximumRunCost"),
    preferLocal:
      document.querySelector<HTMLInputElement>("#preferLocal")?.checked ?? false,
    fallbackAllowed:
      document.querySelector<HTMLInputElement>("#fallbackAllowed")?.checked ??
      false,
    askBeforeCostEscalation:
      document.querySelector<HTMLInputElement>("#askCostEscalation")?.checked ??
      false,
  };
}

function populateModelSelectionSettings(): void {
  const assignment = modelSelectionSettings.orchestration;
  const values: Record<string, string> = {
    orchestrationProvider: assignment.providerConfigId,
    orchestrationModel: assignment.modelId,
    orchestrationEffort: assignment.reasoningEffort,
    orchestrationContext: assignment.maximumContextTokens?.toString() ?? "",
    orchestrationBudget: assignment.maximumBudgetMicrousd?.toString() ?? "",
    orchestrationFallbackProvider:
      assignment.fallbackProviderConfigId ?? "",
    orchestrationFallbackModel: assignment.fallbackModelId ?? "",
  };
  for (const [id, value] of Object.entries(values)) {
    const input = document.querySelector<
      HTMLInputElement | HTMLSelectElement
    >(`#${id}`);
    if (input) input.value = value;
  }
  const locked =
    document.querySelector<HTMLInputElement>("#orchestrationLocked");
  if (locked) locked.checked = assignment.locked;
  const pool = document.querySelector<HTMLElement>("#workerModelPool");
  if (pool) pool.innerHTML = workerPoolMarkup();
}

function readModelSelectionSettings(): ModelSelectionSettings {
  const workerPool = Array.from(
    document.querySelectorAll<HTMLElement>("[data-worker-model-row]"),
  )
    .map((row): ModelAssignment | null => {
      const providerConfigId =
        row.querySelector<HTMLInputElement>("[data-worker-provider]")?.value.trim() ??
        "";
      const modelId =
        row.querySelector<HTMLInputElement>("[data-worker-model]")?.value.trim() ??
        "";
      if (!providerConfigId && !modelId) return null;
      return {
        role: row.dataset.workerModelRow as ModelRole,
        providerConfigId,
        modelId,
        reasoningEffort:
          row.querySelector<HTMLSelectElement>("[data-worker-effort]")?.value as
            | "low"
            | "medium"
            | "high",
        maximumContextTokens: null,
        maximumBudgetMicrousd: null,
        fallbackProviderConfigId: null,
        fallbackModelId: null,
        locked:
          row.querySelector<HTMLInputElement>("[data-worker-locked]")?.checked ??
          false,
      };
    })
    .filter((assignment): assignment is ModelAssignment => assignment !== null);
  const fallbackProvider = inputValue("orchestrationFallbackProvider");
  const fallbackModel = inputValue("orchestrationFallbackModel");
  return {
    orchestration: {
      role: "orchestration",
      providerConfigId: inputValue("orchestrationProvider"),
      modelId: inputValue("orchestrationModel"),
      reasoningEffort: inputValue("orchestrationEffort") as
        | "low"
        | "medium"
        | "high",
      maximumContextTokens: optionalNumber("orchestrationContext"),
      maximumBudgetMicrousd: optionalNumber("orchestrationBudget"),
      fallbackProviderConfigId: fallbackProvider || null,
      fallbackModelId: fallbackModel || null,
      locked:
        document.querySelector<HTMLInputElement>("#orchestrationLocked")
          ?.checked ?? false,
    },
    workerPool,
  };
}

async function loadNativeProviderSettings(): Promise<void> {
  if (!isTauri()) {
    setSettingsStatus(
      "providerActionStatus",
      "浏览器预览不会读取 Credential Manager；请在 LunaScope 桌面应用中配置。",
    );
    renderProviderList();
    renderRoutingPolicy();
    populateStructuredPolicy();
    populateModelSelectionSettings();
    return;
  }
  try {
    const [configs, policy, selections] = await Promise.all([
      invoke<ProviderConfig[]>("list_provider_configs"),
      invoke<ModelRoutingPolicy | null>("get_routing_policy", {
        scopeId: globalRoutingScope,
      }),
      invoke<ModelSelectionSettings | null>("get_model_selection_settings", {
        scopeId: globalRoutingScope,
      }),
    ]);
    providerConfigs = configs;
    if (policy) routingPolicy = policy;
    if (selections) modelSelectionSettings = selections;
    renderProviderList();
    renderRoutingPolicy();
    populateStructuredPolicy();
    populateModelSelectionSettings();
  } catch (error) {
    setSettingsStatus(
      "providerActionStatus",
      `读取本机 Provider 设置失败：${errorMessage(error)}`,
      "error",
    );
  }
}

function skillsMcpMarkup(): string {
  return `<div style="padding:18px 22px;max-width:1040px">
    <div class="pane-head" style="padding-inline:0"><div><strong>Skills, Tools &amp; MCP</strong><span class="settings-help">使用 LunaScope 固定全局目录，不随项目 workspace 切换。外部进程、网络与密钥使用始终经过 Policy Engine。</span></div><button class="ghost-btn" type="button" data-native-action="refresh-extensions">刷新目录</button></div>
    <div class="routing-review">ROOT · ${escapeHtml(extensionDirectories.root)}
SYSTEM SKILLS · ${escapeHtml(extensionDirectories.systemSkills)}
USER SKILLS · ${escapeHtml(extensionDirectories.userSkills)}
TOOLS · ${escapeHtml(extensionDirectories.tools)}
MCP · ${escapeHtml(extensionDirectories.mcp)}</div>
    <div class="settings-status" id="extensionActionStatus" role="status"></div>

    <section style="margin-top:18px"><div class="pane-head" style="padding-inline:0"><div><strong>Skill Catalog</strong><span class="settings-help">系统 Skill 与用户 Skill 独立存放；兼容格式在导入时归一化，兼容性不等于允许执行。</span></div><span class="mono" id="skillCatalogCount">0</span></div><div id="skillCatalogList"><span class="settings-help">正在发现摘要…</span></div><div id="loadedSkillDetail"></div></section>

    <section style="margin-top:24px" aria-labelledby="githubImportHeading">
      <div class="pane-head" style="padding-inline:0"><div><strong id="githubImportHeading">${tr("从 GitHub 导入 Skill", "Import a Skill from GitHub")}</strong><span class="settings-help">${tr("直接粘贴 repository、tree、branch、tag、commit 或子目录链接。内容会固定到完整 commit SHA，并先进入隔离区检查；不会 checkout 或执行。", "Paste a repository, tree, branch, tag, commit, or subdirectory URL. Content is pinned to a full commit SHA and inspected in quarantine; it is never checked out or executed.")}</span></div></div>
      <div class="field"><label for="githubImportUrl">GitHub Skill URL</label><div class="row"><input class="input" id="githubImportUrl" type="url" placeholder="https://github.com/owner/repo/tree/main/path/to/skill" spellcheck="false" style="flex:1"><button class="primary-btn" type="button" data-native-action="preview-github-import">${tr("检查链接", "Inspect URL")}</button></div></div>
      <div class="settings-status" id="githubImportStatus" role="status"></div>
      <div id="githubImportPreview"><span class="settings-help">No repository has been downloaded.</span></div>
      <div class="pane-head" style="padding-inline:0;margin-top:16px"><div><strong>Installed versions</strong><span class="settings-help">Each approved update retains an immutable snapshot for comparison and rollback.</span></div><button class="ghost-btn" type="button" data-native-action="refresh-imports">Refresh</button></div>
      <div id="installedImportList"><span class="settings-help">Reading local import manifests…</span></div>
      <div id="githubImportComparison"></div>
    </section>

    <section style="margin-top:24px"><div class="pane-head" style="padding-inline:0"><div><strong>MCP Servers</strong><span class="settings-help">原生支持 stdio 与 Streamable HTTP。Authorization 等敏感值必须引用 Windows Credential Manager。</span></div></div>
      <form id="mcpServerForm" autocomplete="off">
        <div class="settings-grid">
          <div class="field"><label for="mcpServerId">配置 ID</label><input class="input" id="mcpServerId" value="docs-local" required pattern="[A-Za-z0-9._-]+"></div>
          <div class="field"><label for="mcpServerName">显示名称</label><input class="input" id="mcpServerName" value="Documentation MCP" required></div>
          <div class="field"><label for="mcpTransportKind">Transport</label><select class="select" id="mcpTransportKind"><option value="streamable_http">Streamable HTTP</option><option value="stdio">stdio</option></select></div>
          <div class="field"><label for="mcpTimeout">Timeout (ms)</label><input class="input" id="mcpTimeout" type="number" min="1" max="600000" value="30000"></div>
          <div class="field wide"><label for="mcpTarget">URL 或绝对可执行文件路径</label><input class="input" id="mcpTarget" value="https://example.com/mcp" required spellcheck="false"></div>
          <div class="field wide"><label for="mcpArgs">stdio args（JSON 数组）</label><textarea class="textarea" id="mcpArgs" spellcheck="false" style="min-height:58px">[]</textarea></div>
          <div class="field"><label for="mcpCwd">stdio cwd（绝对路径，可选）</label><input class="input" id="mcpCwd" spellcheck="false"></div>
          <div class="field"><label for="mcpEnabled">状态</label><label><input id="mcpEnabled" type="checkbox" checked> Enabled</label></div>
          <div class="field wide"><label for="mcpEnvironment">stdio environment（JSON 对象，值为 literal / credential_reference）</label><textarea class="textarea" id="mcpEnvironment" spellcheck="false" style="min-height:74px">{}</textarea></div>
          <div class="field wide"><label for="mcpHeaders">HTTP headers（JSON 数组，值为 literal / credential_reference）</label><textarea class="textarea" id="mcpHeaders" spellcheck="false" style="min-height:74px">[]</textarea></div>
          <div class="field"><label for="mcpCredentialReference">本次写入的凭据引用 ID（可选）</label><input class="input" id="mcpCredentialReference" value="bearer" pattern="[A-Za-z0-9._-]+"></div>
          <div class="field"><label for="mcpCredentialSecret">Secret（留空不修改；不会回显）</label><input class="input" id="mcpCredentialSecret" type="password" autocomplete="new-password" spellcheck="false"></div>
        </div>
        <div class="row"><button class="primary-btn" type="submit">保存 MCP 配置</button></div>
      </form>
      <div class="settings-status" id="mcpActionStatus" role="status"></div>
      <div id="mcpServerList"><span class="settings-help">正在读取本机配置…</span></div>
    </section>

    <section style="margin-top:24px"><div class="pane-head" style="padding-inline:0"><div><strong>Tool Routing Preview</strong><span class="settings-help">Planner / Reviewer 默认只读；普通 Worker 最多路由 1–3 类工具。Ask 与 Deny 不会静默变成 Allow。</span></div></div>
      <div class="settings-grid">
        <div class="field"><label for="routeRole">Worker role</label><select class="select" id="routeRole"><option value="builder">Builder</option><option value="researcher">Researcher</option><option value="planner">Planner</option><option value="reviewer">Reviewer</option></select></div>
        <div class="field"><label for="routeMaxTools">Max tool classes</label><input class="input" id="routeMaxTools" type="number" min="1" max="3" value="3"></div>
        <div class="field wide"><label for="routeObjective">Objective</label><input class="input" id="routeObjective" value="研究并实现一个安全的代码修复"></div>
        <div class="field"><label for="routePreferredTools">Preferred tool IDs（逗号分隔）</label><input class="input" id="routePreferredTools" value=""></div>
        <div class="field"><label for="routeSkills">Required skill catalog IDs（逗号分隔）</label><input class="input" id="routeSkills" value=""></div>
      </div>
      <button class="ghost-btn" type="button" data-native-action="route-tools">评估工具路由</button>
      <div class="settings-status" id="toolRouteStatus" role="status"></div>
      <div id="toolRoutingResult"></div>
    </section>
  </div>`;
}

function hydrateIntegrationHub(): void {
  const panel =
    document.querySelector<HTMLElement>("#nativeIntegrationsPanel");
  if (!panel || panel.dataset.nativeHydrated === "true") return;
  panel.dataset.nativeHydrated = "true";
  panel.innerHTML = skillsMcpMarkup();
  void loadExtensionsWorkspace();
}

function nativeWorkerGraphMarkup(
  plan: OrchestrationSession["plan"],
  states: Record<string, string>,
): string {
  const workersById = new Map(
    plan.workers.map((worker) => [worker.workerId, worker]),
  );
  const levelCache = new Map<string, number>();
  const levelFor = (worker: WorkerSpec, visiting = new Set<string>()): number => {
    const cached = levelCache.get(worker.workerId);
    if (cached !== undefined) return cached;
    if (visiting.has(worker.workerId)) return 0;
    visiting.add(worker.workerId);
    const level =
      worker.dependencies.length === 0
        ? 0
        : 1 +
          Math.max(
            0,
            ...worker.dependencies.map((dependency) => {
              const upstream = workersById.get(dependency);
              return upstream ? levelFor(upstream, visiting) : 0;
            }),
          );
    visiting.delete(worker.workerId);
    levelCache.set(worker.workerId, level);
    return level;
  };
  const levels = new Map<number, WorkerSpec[]>();
  plan.workers.forEach((worker) => {
    const level = levelFor(worker);
    levels.set(level, [...(levels.get(level) ?? []), worker]);
  });
  const maximumLevel = Math.max(0, ...levels.keys());
  const maximumRows = Math.max(1, ...[...levels.values()].map((items) => items.length));
  const planeHeight = Math.max(320, 44 + maximumRows * 154);
  const workerWidth = 216;
  const workerHeight = 124;
  const positions = new Map<string, { x: number; y: number }>();
  for (const [level, workers] of levels) {
    const occupied = workers.length * workerHeight + (workers.length - 1) * 34;
    const startY = Math.max(24, (planeHeight - occupied) / 2);
    workers.forEach((worker, row) => {
      positions.set(worker.workerId, {
        x: 314 + level * 292,
        y: startY + row * (workerHeight + 34),
      });
    });
  }
  const synthesisX = 314 + (maximumLevel + 1) * 292;
  const planeWidth = synthesisX + 270;
  const centerY = (planeHeight - workerHeight) / 2;
  const roots = plan.workers.filter((worker) => worker.dependencies.length === 0);
  const dependedOn = new Set(plan.workers.flatMap((worker) => worker.dependencies));
  const leaves = plan.workers.filter(
    (worker) => !dependedOn.has(worker.workerId),
  );
  const curve = (
    from: { x: number; y: number },
    to: { x: number; y: number },
  ) => {
    const startX = from.x + workerWidth;
    const startY = from.y + workerHeight / 2;
    const endX = to.x;
    const endY = to.y + workerHeight / 2;
    const bend = Math.max(48, (endX - startX) / 2);
    return `M ${startX} ${startY} C ${startX + bend} ${startY}, ${endX - bend} ${endY}, ${endX} ${endY}`;
  };
  const orchestrator = { x: 30, y: centerY };
  const synthesis = { x: synthesisX, y: centerY };
  const edges = [
    ...roots.map((worker) => ({
      path: curve(orchestrator, positions.get(worker.workerId)!),
      synthesis: false,
    })),
    ...plan.workers.flatMap((worker) =>
      worker.dependencies.flatMap((dependency) => {
        const from = positions.get(dependency);
        const to = positions.get(worker.workerId);
        return from && to ? [{ path: curve(from, to), synthesis: false }] : [];
      }),
    ),
    ...leaves.map((worker) => ({
      path: curve(positions.get(worker.workerId)!, synthesis),
      synthesis: true,
    })),
  ];
  const phaseMarkup = [...levels.entries()]
    .sort(([left], [right]) => left - right)
    .map(([level, workers]) => {
      const roles = [...new Set(workers.map((worker) => worker.role))]
        .map(orchestrationRoleLabel)
        .join(" / ")
        .toUpperCase();
      return `<div class="native-graph-phase" style="left:${286 + level * 292}px;top:12px;width:260px;height:${planeHeight - 24}px"><span>P${level + 1} · ${escapeHtml(roles)}</span></div>`;
    })
    .join("");
  const workerNodes = plan.workers
    .map((worker, index) => {
      const position = positions.get(worker.workerId)!;
      const state = states[worker.workerId] ?? "planned";
      const running = ["running_model", "running_tool", "verifying", "recovering", "retrying", "localizing", "repair_planning"].includes(state);
      return `<button class="native-graph-node ${running ? "is-running" : ""}" type="button" style="left:${position.x}px;top:${position.y}px" data-native-action="focus-orchestration-worker" data-orchestration-worker-index="${index}" data-native-worker-id="${escapeHtml(worker.workerId)}" aria-label="${escapeHtml(tr(`查看${orchestrationRoleLabel(worker.role)}子 Agent`, `Inspect ${worker.role} Worker`))}">
        <span class="native-graph-node-head"><span><span class="native-graph-node-id">${escapeHtml(worker.workerId)}</span><span class="native-graph-node-role">${escapeHtml(orchestrationRoleLabel(worker.role))}</span></span><span class="node-status">${escapeHtml(orchestrationStateLabel(state))}</span></span>
        <span class="native-graph-node-body">${escapeHtml(localizedModelProse(worker.task,"执行本节点职责并完成对应交付物与验收标准。","Execute this node and its acceptance criteria."))}${workerTagMarkup(worker.tags)}</span>
        <span class="native-graph-node-foot"><span>${worker.dependencies.length ? tr("等待前置结果","DEPENDENCY") : tr("可调度","READY")}</span><span>${escapeHtml(localizedModelProse(worker.expectedOutput,"真实交付物与验收证据。","Deliverable and evidence."))}</span></span>
      </button>`;
    })
    .join("");
  const minimapNodes = [
    orchestrator,
    ...positions.values(),
    synthesis,
  ]
    .map(
      (position) =>
        `<i style="left:${Math.round((position.x / planeWidth) * 136)}px;top:${Math.round((position.y / planeHeight) * 70)}px"></i>`,
    )
    .join("");
  return `<section class="native-graph-shell" aria-label="${tr("持久化子 Agent 依赖图","Durable Worker dependency graph")}">
    <div class="native-graph-scroll"><div class="native-graph-plane" style="width:${planeWidth}px;height:${planeHeight}px">
      ${phaseMarkup}
      <svg viewBox="0 0 ${planeWidth} ${planeHeight}" width="${planeWidth}" height="${planeHeight}" aria-hidden="true">${edges.map((edge) => `<path class="native-graph-edge ${edge.synthesis ? "synthesis" : ""}" d="${edge.path}"></path>`).join("")}</svg>
      <button class="native-graph-node orchestrator" type="button" style="left:${orchestrator.x}px;top:${orchestrator.y}px" data-native-action="inspect-orchestration-node" data-orchestration-node="orchestrator">
        <span class="native-graph-node-head"><span><span class="native-graph-node-id">${tr("编排器","ORCHESTRATOR")}</span><span class="native-graph-node-role">${tr("计划与调度","Plan & dispatch")}</span></span><span class="node-status">${plan.decision.kind==="multi_agent"?tr("多 Agent","MULTI-AGENT"):tr("单 Agent","SINGLE-AGENT")}</span></span>
        <span class="native-graph-node-body">${escapeHtml(localizedModelProse(plan.decision.rationale,"已根据任务复杂度生成可执行编排。","Executable orchestration generated."))}${workerTagMarkup([plan.domainPack ?? tr("自动领域","auto-domain"), tr("用户约束","user constraints"), tr("模型池","model pool")])}</span>
        <span class="native-graph-node-foot"><span>${tr("蓝图","BLUEPRINT")}</span><span>${tr("图","GRAPH")} V${plan.version}</span></span>
      </button>
      ${workerNodes}
      <button class="native-graph-node synthesis" type="button" style="left:${synthesis.x}px;top:${synthesis.y}px" data-native-action="inspect-orchestration-node" data-orchestration-node="synthesis">
        <span class="native-graph-node-head"><span><span class="native-graph-node-id">${tr("编排器","ORCHESTRATOR")}</span><span class="native-graph-node-role">${tr("汇总","Synthesis")}</span></span><span class="node-status">${nativeOrchestrationResult ? escapeHtml(orchestrationStateLabel(nativeOrchestrationResult.verification.status==="verified"?"completed":"verifying")) : tr("等待","PENDING")}</span></span>
        <span class="native-graph-node-body">${tr("汇总不可变交接物与独立验收记录。","Aggregate immutable handoffs and the independent verification record.")}${workerTagMarkup([tr("交接物","handoffs"),tr("证据","evidence")])}</span>
        <span class="native-graph-node-foot"><span>${tr("汇总","SYNTHESIS")}</span><span>${tr("证据","EVIDENCE")}</span></span>
      </button>
    </div></div>
    <div class="native-graph-minimap" aria-hidden="true">${minimapNodes}</div>
  </section>`;
}

function nativeOrchestrationMarkup(): string {
  return `<header class="orch-toolbar native-orch-toolbar">
      <div class="segments"><button class="seg-btn active" type="button">${tr("蓝图","Blueprint")}</button></div>
      <span class="spacer"></span>
      <button class="text-btn" type="button" data-native-action="native-orchestration-layout">${tr("自动布局","Auto Layout")}</button>
      <button class="text-btn" type="button" data-native-action="native-orchestration-fit">${tr("适应视图","Fit")}</button>
      <button class="text-btn" type="button" data-native-action="native-orchestration-focus">${tr("专注","Focus")}</button>
      <span class="mono" id="nativeZoomLabel">100%</span>
    </header>
    <div class="orch-stage native-orch-stage">
      <section id="nativeOrchestrationPlan"></section>
    </div>`;
}

async function hydrateOrchestrationHub(): Promise<void> {
  const panel = document.querySelector<HTMLElement>(
    '[data-od-id="orchestration-workspace"]',
  );
  if (!panel || panel.dataset.nativeHydrated === "true") return;
  panel.dataset.nativeHydrated = "true";
  if (isTauri()) {
    try {
      [providerConfigs, domainPacks] = await Promise.all([
        invoke<ProviderConfig[]>("list_provider_configs"),
        invoke<DomainPackDescriptor[]>("list_domain_packs"),
      ]);
    } catch (error) {
      panel.innerHTML = nativeOrchestrationMarkup();
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `Provider discovery failed: ${errorMessage(error)}`,
        "error",
      );
      return;
    }
  }
  panel.innerHTML = nativeOrchestrationMarkup();
  renderNativeOrchestration();
}

function nativeOrchestrationModeMarkup(
  session: OrchestrationSession | null,
): string {
  if (!session) {
    return '<article class="provider-card"><span class="settings-help">Draft a durable graph in Blueprint before opening this control mode.</span></article>';
  }
  const plan = session.plan;
  if (nativeOrchestrationMode === "runtime") {
    return `<div class="pane-head"><div><strong>Runtime</strong><span class="settings-help">Durable Worker states from the native scheduler.</span></div><span class="mono">GRAPH V${plan.version}</span></div>
      <div class="native-runtime-list">${plan.workers
        .map(
          (worker) =>
            `<button class="native-runtime-row" type="button" data-native-action="focus-orchestration-worker" data-orchestration-worker-index="${plan.workers.indexOf(worker)}"><span class="dot ${session.snapshot.workers[worker.workerId] ?? "draft"}"></span><span><strong>${escapeHtml(worker.role)}</strong><small>${escapeHtml(worker.task)}</small>${workerTagMarkup(worker.tags)}</span><span class="worker-runtime"><strong>${escapeHtml(session.snapshot.workers[worker.workerId] ?? "draft")}</strong><small>${worker.dependencies.length ? "waiting on dependency boundary" : "ready for dispatch"}</small></span></button>`,
        )
        .join("")}</div>`;
  }
  if (nativeOrchestrationMode === "queue") {
    return `<div class="pane-head"><div><strong>Queue</strong><span class="settings-help">Dispatch order follows the durable dependency graph; concurrency remains capped at ${plan.maximumParallelWorkers}.</span></div><span class="mono">${plan.workers.length} WORKERS</span></div>
      <div class="queue-columns"><section class="queue-column"><h3>Ready</h3>${plan.workers
        .filter((worker) => worker.dependencies.length === 0)
        .map(
          (worker) =>
            `<div class="queue-card"><strong>${escapeHtml(worker.role)}</strong><span>${escapeHtml(worker.workerId)}</span>${workerTagMarkup(worker.tags)}</div>`,
        )
        .join("")}</section><section class="queue-column"><h3>Dependency-bound</h3>${plan.workers
        .filter((worker) => worker.dependencies.length > 0)
        .map(
          (worker) =>
            `<div class="queue-card"><strong>${escapeHtml(worker.role)}</strong><span>${escapeHtml(worker.dependencies.join(" → "))}</span>${workerTagMarkup(worker.tags)}</div>`,
        )
        .join("")}</section></div>`;
  }
  const handoffs = plan.workers.flatMap((worker) =>
    worker.dependencies.map((dependency) => ({
      from: dependency,
      to: worker.workerId,
      payload: worker.inputContext.artifactIds.join(", ") || "structured output",
      status:
        session.snapshot.workers[worker.workerId] === "completed"
          ? "consumed"
          : "planned",
    })),
  );
  return `<div class="pane-head"><div><strong>Handoffs</strong><span class="settings-help">Immutable dependency and evidence boundaries.</span></div><span class="mono">${handoffs.length} EDGES</span></div>
    <table class="table"><thead><tr><th>FROM</th><th>TO</th><th>PAYLOAD</th><th>STATUS</th></tr></thead><tbody>${handoffs
      .map(
        (handoff) =>
          `<tr><td class="mono">${escapeHtml(handoff.from)}</td><td class="mono">${escapeHtml(handoff.to)}</td><td>${escapeHtml(handoff.payload)}</td><td>${escapeHtml(handoff.status)}</td></tr>`,
      )
      .join("")}</tbody></table>`;
}

function applyNativeOrchestrationMode(): void {
  const blueprint = nativeOrchestrationMode === "blueprint";
  const draftPanel = document.querySelector<HTMLElement>(
    "#nativeOrchestrationDraftPanel",
  );
  const planPanel = document.querySelector<HTMLElement>(
    "#nativeOrchestrationPlan",
  );
  const addPanel = document.querySelector<HTMLElement>(
    "#nativeOrchestrationAddPanel",
  );
  const resultPanel = document.querySelector<HTMLElement>(
    "#nativeOrchestrationResult",
  );
  const modePanel = document.querySelector<HTMLElement>(
    "#nativeOrchestrationModePanel",
  );
  if (draftPanel)
    draftPanel.hidden = !blueprint || Boolean(nativeOrchestrationSession);
  if (planPanel) planPanel.hidden = !blueprint;
  if (addPanel) addPanel.hidden = !blueprint;
  if (resultPanel) resultPanel.hidden = !blueprint;
  if (modePanel) {
    modePanel.hidden = blueprint;
    modePanel.innerHTML = blueprint
      ? ""
      : nativeOrchestrationModeMarkup(nativeOrchestrationSession);
  }
  document
    .querySelectorAll<HTMLButtonElement>("[data-native-mode]")
    .forEach((button) =>
      button.classList.toggle(
        "active",
        button.dataset.nativeMode === nativeOrchestrationMode,
      ),
    );
}

function renderNativeOrchestration(): void {
  window.lunaScopeUi?.setRuntimeProjection({
    plan: nativeProjectionPlan,
    result: nativeOrchestrationResult,
    changeSets: nativeChangeSets,
    agentPlans: nativeAgentPlans,
  });
  const panel = document.querySelector<HTMLElement>(
    "#nativeOrchestrationPlan",
  );
  if (!panel) return;
  const session = nativeOrchestrationSession;
  if (!session) {
    panel.innerHTML =
      `<article class="native-orch-empty"><strong>${tr("尚无编排图", "No orchestration graph yet")}</strong><span class="settings-help">${tr("回到项目对话并发送任务，Orchestration Model 会自动判断并生成单 Agent 或多 Agent 图。", "Return to the project conversation and send a request. The Orchestration Model will automatically create a single-agent or multi-agent graph.")}</span></article>`;
    return;
  }
  panel.innerHTML = nativeWorkerGraphMarkup(
    session.plan,
    session.snapshot.workers,
  );
}

function renderNativeOrchestrationResult(): void {
  const panel = document.querySelector<HTMLElement>(
    "#nativeOrchestrationResult",
  );
  if (!panel) return;
  const result = nativeOrchestrationResult;
  if (!result) {
    panel.innerHTML = "";
    return;
  }
  const workers = Object.values(result.workers)
    .map(
      (worker) =>
        `${worker.workerId} · ${orchestrationStateLabel(worker.state)} · ${tr("尝试", "attempts")} ${worker.attempts} · ${localizedModelProse(worker.summary, "本节点已完成；原始证据保存在交付物中。", "Node completed with stored evidence.")}`,
    )
    .join("\n");
  const handoffs = result.handoffs
    .map(
      (handoff) =>
        `${handoff.fromWorkerId} → ${handoff.toWorkerId} · ${handoff.artifactIds.length} ${tr("个交付物", "artifacts")}`,
    )
    .join("\n");
  panel.innerHTML = `<article class="provider-card"><header><div><strong>${tr("编排汇总", "Orchestrator synthesis")}</strong><small>${escapeHtml(result.orchestrationId)} · v${result.version}</small></div><span class="status ${result.verification.status === "verified" ? "complete" : "waiting"}">${escapeHtml(result.verification.status === "verified" ? tr("已验证", "VERIFIED") : tr("待确认", "NEEDS REVIEW"))}</span></header>
    <div class="routing-review" style="margin-top:10px">${escapeHtml(localizedModelProse(result.synthesis, "所有节点已结束；详细原始证据保存在各子 Agent 交付物中。", "All nodes finished with stored evidence."))}\n\n${tr("子 Agent", "Workers")}\n${escapeHtml(workers)}\n\n${tr("交接", "Handoffs")}\n${escapeHtml(handoffs || tr("无", "none"))}\n\n${tr("验收", "Verification")}\n${escapeHtml(localizedModelProse(result.verification.summary, "独立验收已完成。", "Independent verification completed."))}\n${tr("证据", "Evidence")}\n${escapeHtml(result.verification.evidence.join("\n") || tr("无", "none"))}\n${tr("剩余风险", "Remaining risks")}\n${escapeHtml(result.verification.remainingRisks.map((risk) => localizedModelProse(risk, "存在尚未完全消除的验收风险。", "An unresolved verification risk remains.")).join("\n") || tr("无", "none"))}</div>
    <small>${result.artifacts.length} ${tr("个不可变交付物", "immutable artifacts")} · ${result.stateChanges.length} ${tr("次持久化状态变更", "durable state transitions")}</small>
  </article>`;
}

function openNativeInspector(
  kind: "orchestrator" | "worker" | "synthesis",
  workerIndex?: number,
): void {
  nativeInspector = { kind, workerIndex };
  nativeInspectorTab = "overview";
  renderNativeInspector();
}

function closeNativeInspector(): void {
  nativeInspector = null;
  document.querySelector("#app")?.classList.remove("review-open");
  document.querySelector("#review")?.setAttribute("aria-hidden", "true");
}

function workerInspectorMarkup(worker: WorkerSpec, index: number): string {
  const actionBar = `<div class="contextual-actions"><button class="primary-btn" type="button" data-native-action="patch-orchestration-worker" data-orchestration-worker-index="${index}">Apply user Patch</button><button class="text-btn" type="button" data-native-action="remove-orchestration-worker" data-orchestration-worker-index="${index}">Remove Draft Worker</button></div>`;
  if (nativeInspectorTab === "prompt") {
    return `${actionBar}<section class="inspect-section"><h3>WORKER PROMPT</h3><textarea class="textarea" id="orchPrompt${index}" style="min-height:360px">${escapeHtml(worker.prompt)}</textarea><label><input type="checkbox" id="orchLockPrompt${index}" ${worker.lockedFields.includes("prompt") ? "checked" : ""}> Lock Prompt</label></section>`;
  }
  if (nativeInspectorTab === "context") {
    return `<section class="inspect-section"><h3>INPUT CONTEXT</h3><p>${escapeHtml(worker.inputContext.summary)}</p><dl class="definition"><div><dt>Workspace snapshot</dt><dd>${worker.inputContext.includeWorkspaceSnapshot ? "Included" : "Excluded"}</dd></div><div><dt>Dependencies</dt><dd>${escapeHtml(worker.dependencies.join(", ") || "none")}</dd></div></dl></section>`;
  }
  if (nativeInspectorTab === "tools") {
    const options = providerConfigs
      .filter((provider) => provider.enabled)
      .map(
        (provider) =>
          `<option value="${escapeHtml(provider.id)}" ${provider.id === worker.model.provider ? "selected" : ""}>${escapeHtml(provider.displayName)}</option>`,
      )
      .join("");
    return `${actionBar}<section class="inspect-section"><h3>MODEL, TOOLS &amp; SKILLS</h3><div class="field"><label>Provider</label><select class="select" id="orchProvider${index}">${options}</select></div><div class="field"><label>Model</label><input class="input" id="orchModel${index}" value="${escapeHtml(worker.model.model)}"></div><div class="field"><label>Tools · CSV</label><input class="input" id="orchTools${index}" value="${escapeHtml(worker.tools.join(", "))}"></div><div class="field"><label>Skills · CSV</label><input class="input" id="orchSkills${index}" value="${escapeHtml(worker.skills.join(", "))}"></div><div class="field"><label>Dependencies · CSV Worker IDs</label><input class="input" id="orchDependencies${index}" value="${escapeHtml(worker.dependencies.join(", "))}"></div><div class="field"><label>Write scopes · CSV</label><input class="input" id="orchWriteScopes${index}" value="${escapeHtml(worker.writeScopes.join(", "))}"></div></section>`;
  }
  if (nativeInspectorTab === "permissions") {
    return `${actionBar}<section class="inspect-section"><h3>PERMISSION POLICY</h3><div class="field"><label>Permissions · CSV</label><input class="input" id="orchPermissions${index}" value="${escapeHtml(worker.permissions.join(", "))}"></div><p class="settings-help">Agent permissions follow the conversation access mode. Rust hard-deny rules remain in force.</p></section>`;
  }
  if (nativeInspectorTab === "output") {
    return `<section class="inspect-section"><h3>OUTPUT CONTRACT</h3><dl class="definition"><div><dt>Expected output</dt><dd>${escapeHtml(worker.expectedOutput)}</dd></div><div><dt>Media type</dt><dd>${escapeHtml(worker.outputSchema.mediaType)}</dd></div><div><dt>Criteria</dt><dd>${escapeHtml(worker.completionCriteria.join(" · "))}</dd></div></dl></section>`;
  }
  if (nativeInspectorTab === "runtime") {
    return `<section class="inspect-section"><h3>RUNTIME</h3><dl class="definition"><div><dt>State</dt><dd>${escapeHtml(nativeOrchestrationSession?.snapshot.workers[worker.workerId] ?? "draft")}</dd></div><div><dt>Timeout</dt><dd>${worker.timeoutMs} ms</dd></div><div><dt>Depth</dt><dd>${worker.depth}</dd></div></dl></section>`;
  }
  if (nativeInspectorTab === "history") {
    return `<section class="inspect-section"><h3>CHANGE HISTORY</h3><p>Graph v${nativeOrchestrationSession?.plan.version ?? 1} · ${nativeOrchestrationSession?.plan.userOverrides.length ?? 0} user overrides.</p></section>`;
  }
  return `${actionBar}<section class="inspect-section"><h3>WORKER SPEC</h3><div class="field"><label>Role</label><input class="input" id="orchRole${index}" list="nativeWorkerRoleCatalog" value="${escapeHtml(worker.role)}"></div><div class="field"><label>Labels · CSV</label><input class="input" id="orchTags${index}" value="${escapeHtml(worker.tags.join(", "))}"></div><div class="field"><label>Task</label><textarea class="textarea" id="orchTask${index}" style="min-height:140px">${escapeHtml(worker.task)}</textarea></div><div class="field"><label>Patch timing</label><select class="select" id="orchPatchMode${index}"><option value="apply_now">Apply now</option><option value="apply_after_current_step">After current step</option><option value="apply_on_retry">On retry</option><option value="clone_revision">Clone revision</option><option value="cancel">Cancel active run</option></select></div><div class="row"><label><input type="checkbox" id="orchLockRole${index}" ${worker.lockedFields.includes("role") ? "checked" : ""}> Lock Role</label><label><input type="checkbox" id="orchLockTags${index}" ${worker.lockedFields.includes("tags") ? "checked" : ""}> Lock Labels</label></div></section>`;
}

function renderNativeInspector(): void {
  const inspector = nativeInspector;
  const session = nativeOrchestrationSession;
  if (!inspector || !session) return;
  const app = document.querySelector("#app");
  const review = document.querySelector<HTMLElement>("#review");
  const title = document.querySelector<HTMLElement>("#reviewTitle");
  const tabs = document.querySelector<HTMLElement>("#reviewTabs");
  const body = document.querySelector<HTMLElement>("#reviewBody");
  if (!app || !review || !title || !tabs || !body) return;
  app.classList.add("review-open");
  review.setAttribute("aria-hidden", "false");
  if (inspector.kind === "worker") {
    const index = inspector.workerIndex ?? 0;
    const worker = session.plan.workers[index];
    if (!worker) return;
    title.textContent = `${worker.workerId} · Worker Inspector`;
    const names = [
      "overview",
      "prompt",
      "context",
      "tools",
      "permissions",
      "output",
      "runtime",
      "history",
    ];
    tabs.innerHTML = names
      .map(
        (name) =>
          `<button class="${nativeInspectorTab === name ? "active" : ""}" type="button" data-native-action="native-inspector-tab" data-inspector-tab="${name}">${name[0].toUpperCase() + name.slice(1)}</button>`,
      )
      .join("");
    body.innerHTML = workerInspectorMarkup(worker, index);
    return;
  }
  if (inspector.kind === "orchestrator") {
    title.textContent = "Orchestrator Inspector";
    tabs.innerHTML = ["overview", "plan", "runtime", "history"]
      .map(
        (name) =>
          `<button class="${nativeInspectorTab === name ? "active" : ""}" type="button" data-native-action="native-inspector-tab" data-inspector-tab="${name}">${name[0].toUpperCase() + name.slice(1)}</button>`,
      )
      .join("");
    const cancellation = nativeOrchestrationRunning
      ? '<div class="contextual-actions"><button class="ghost-btn" type="button" data-native-action="cancel-native-orchestration">Cancel</button></div>'
      : "";
    body.innerHTML = `${cancellation}<section class="inspect-section"><h3>${nativeInspectorTab.toUpperCase()}</h3><dl class="definition"><div><dt>Decision</dt><dd>${escapeHtml(session.plan.decision.kind)}</dd></div><div><dt>Rationale</dt><dd>${escapeHtml(session.plan.decision.rationale)}</dd></div><div><dt>Benefit</dt><dd>${escapeHtml(session.plan.decision.expectedBenefit)}</dd></div><div><dt>Graph</dt><dd>v${session.plan.version} · ${session.plan.workers.length} Workers</dd></div><div><dt>Model pool</dt><dd>${escapeHtml(session.plan.allowedWorkerModels.map((model) => `${model.provider}/${model.model}`).join(" · "))}</dd></div></dl></section>`;
    return;
  }
  title.textContent = "Synthesis Inspector";
  tabs.innerHTML = `<button class="active" type="button">Overview</button>`;
  body.innerHTML = nativeOrchestrationResult
    ? `<section class="inspect-section"><h3>VERIFICATION</h3><p>${escapeHtml(nativeOrchestrationResult.synthesis)}</p><dl class="definition"><div><dt>Status</dt><dd>${escapeHtml(nativeOrchestrationResult.verification.status)}</dd></div><div><dt>Evidence</dt><dd>${escapeHtml(nativeOrchestrationResult.verification.evidence.join(" · ") || "none")}</dd></div><div><dt>Remaining risks</dt><dd>${escapeHtml(nativeOrchestrationResult.verification.remainingRisks.join(" · ") || "none")}</dd></div></dl></section>`
    : `<section class="inspect-section"><h3>SYNTHESIS</h3><p>No Worker result has been synthesized yet.</p></section>`;
}

function renderSkillCatalog(): void {
  const list = document.querySelector<HTMLElement>("#skillCatalogList");
  const count = document.querySelector<HTMLElement>("#skillCatalogCount");
  if (count) count.textContent = String(skillSummaries.length);
  if (!list) return;
  if (!skillSummaries.length) {
    list.innerHTML =
      '<article class="provider-card"><span class="settings-help">当前扫描根下未发现 SKILL.md。</span></article>';
    return;
  }
  list.innerHTML = skillSummaries
    .map(
      (skill, index) =>
        `<article class="provider-card"><header><div><strong>${escapeHtml(skill.name)}</strong><small>${escapeHtml(skill.description)}</small><small>${escapeHtml(skill.source)} · ${escapeHtml(skill.compatibility)} · ~${skill.approximateContextTokens} tokens · ${escapeHtml(skill.path)}</small><small>Tools: ${escapeHtml(skill.requiredTools.join(", ") || "none")} · Permissions: ${escapeHtml(skill.requiredPermissions.join(", ") || "none")}</small>${skill.warnings.length ? `<small style="color:var(--orange)">Warnings: ${escapeHtml(skill.warnings.join("; "))}</small>` : ""}</div><span class="status ${skill.compatibility === "unsupported" ? "failed" : skill.compatibility === "bridge_required" ? "waiting" : "complete"}">${escapeHtml(skill.compatibility)}</span></header><div class="row" style="margin-top:10px"><button class="ghost-btn" type="button" data-native-action="load-skill" data-skill-index="${index}">按需加载正文</button><button class="text-btn" type="button" data-native-action="route-with-skill" data-skill-index="${index}">用于路由预览</button></div></article>`,
    )
    .join("");
}

function renderGithubImportPreview(): void {
  const panel = document.querySelector<HTMLElement>("#githubImportPreview");
  if (!panel) return;
  if (!githubImportPreview) {
    panel.innerHTML =
      '<span class="settings-help">No repository has been downloaded.</span>';
    return;
  }
  const preview = githubImportPreview;
  const riskyFiles = preview.files.filter((file) => file.risks.length > 0);
  const components = preview.components
    .map(
      (component) => `<label class="provider-card" style="display:block">
        <input type="checkbox" data-import-component-id="${escapeHtml(component.id)}" ${preview.blocked ? "disabled" : ""}>
        <strong>${escapeHtml(component.displayName)}</strong>
        <small>${escapeHtml(component.kind)} · ${escapeHtml(component.compatibility)} · ${escapeHtml(component.pathPrefix || "(repository root)")}</small>
        <small>Permissions: ${escapeHtml(component.requestedPermissions.join(", ") || "none")}</small>
        ${component.warnings.length ? `<small style="color:var(--orange)">${escapeHtml(component.warnings.join("; "))}</small>` : ""}
      </label>`,
    )
    .join("");
  const findings = riskyFiles
    .slice(0, 30)
    .map(
      (file) =>
        `${file.path} · ${file.risks.join(", ")}${file.extractable ? "" : " · excluded"}`,
    )
    .join("\n");
  const permissionPreview = preview.permissionRequests
    .map((request) => `${request.permission} · ${request.action}`)
    .join("\n");
  panel.innerHTML = `<article class="provider-card" style="margin-top:10px">
    <header><div><strong>${escapeHtml(preview.source.owner)}/${escapeHtml(preview.source.repository)}</strong>
      <small>PINNED COMMIT · ${escapeHtml(preview.commitSha)}</small>
      <small>Content SHA-256 · ${escapeHtml(preview.contentSha256)}</small>
      <small>Subdirectory · ${escapeHtml(preview.source.subdirectory ?? "(repository root)")} · ${preview.files.length} files</small>
      <small>License · ${escapeHtml(preview.license.spdxId ?? preview.license.status)}${preview.license.filePath ? ` · ${escapeHtml(preview.license.filePath)}` : ""}</small>
      <small>Quarantine · ${escapeHtml(preview.quarantinePath)}</small>
    </div><span class="status ${preview.blocked ? "failed" : riskyFiles.length ? "waiting" : "complete"}">${preview.blocked ? "BLOCKED" : "INSPECTED"}</span></header>
    ${preview.warnings.length ? `<div class="routing-review" style="margin-top:10px">${escapeHtml(preview.warnings.join("\n"))}</div>` : ""}
    <div class="pane-head" style="padding-inline:0;margin-top:12px"><strong>Select components</strong><span class="mono">${preview.components.length}</span></div>
    <div>${components}</div>
    <div class="routing-review" style="margin-top:10px">Permission preview\n${escapeHtml(permissionPreview || "none")}\n\nRisk findings (${riskyFiles.length})\n${escapeHtml(findings || "none")}${riskyFiles.length > 30 ? `\n… ${riskyFiles.length - 30} more` : ""}</div>
    <div class="row" style="margin-top:10px"><button class="primary-btn" type="button" data-native-action="install-github-import" ${preview.blocked ? "disabled" : ""}>Approve selected install</button></div>
  </article>`;
}

function renderInstalledImports(): void {
  const list = document.querySelector<HTMLElement>("#installedImportList");
  if (!list) return;
  if (!installedImportVersions.length) {
    list.innerHTML =
      '<article class="provider-card"><span class="settings-help">No approved import versions are installed.</span></article>';
    return;
  }
  list.innerHTML = installedImportVersions
    .map(
      (version, index) => `<article class="provider-card">
        <header><div><strong>${escapeHtml(version.source.owner)}/${escapeHtml(version.source.repository)}</strong>
          <small>${escapeHtml(version.commitSha)} · ${escapeHtml(version.installedAt)}</small>
          <small>${escapeHtml(version.installedPath)} · ${escapeHtml(version.componentIds.join(", "))}</small>
        </div><span class="status ${version.active ? "complete" : "waiting"}">${version.active ? "ACTIVE" : "ROLLBACK"}</span></header>
        <div class="row" style="margin-top:8px">
          ${!version.active ? `<button class="ghost-btn" type="button" data-native-action="rollback-import" data-import-version-index="${index}">Restore this version</button>` : ""}
          ${version.active && githubImportPreview ? `<button class="ghost-btn" type="button" data-native-action="compare-import" data-import-version-index="${index}">Compare with quarantine</button>` : ""}
        </div>
      </article>`,
    )
    .join("");
}

function renderImportComparison(comparison: ImportComparison): void {
  const panel = document.querySelector<HTMLElement>(
    "#githubImportComparison",
  );
  if (!panel) return;
  const lines = (label: string, values: string[]): string =>
    `${label} (${values.length})\n${values.map((value) => `  ${value}`).join("\n") || "  none"}`;
  panel.innerHTML = `<div class="routing-review" style="margin-top:10px">Active ${escapeHtml(comparison.activeCommitSha)}\nCandidate ${escapeHtml(comparison.candidateCommitSha)}\n\n${escapeHtml(lines("Added", comparison.added))}\n\n${escapeHtml(lines("Changed", comparison.changed))}\n\n${escapeHtml(lines("Removed", comparison.removed))}</div>`;
}

async function loadImportState(): Promise<void> {
  if (!isTauri()) {
    renderGithubImportPreview();
    renderInstalledImports();
    return;
  }
  try {
    const versions = await invoke<InstalledImportVersion[]>(
      "list_installed_imports",
    );
    installedImportVersions = versions;
    renderGithubImportPreview();
    renderInstalledImports();
  } catch (error) {
    setSettingsStatus(
      "githubImportStatus",
      `Failed to read import state: ${errorMessage(error)}`,
      "error",
    );
  }
}

function renderMcpServerList(): void {
  const list = document.querySelector<HTMLElement>("#mcpServerList");
  if (!list) return;
  if (!mcpServerConfigs.length) {
    list.innerHTML =
      '<article class="provider-card"><span class="settings-help">尚未保存 MCP Server。</span></article>';
    return;
  }
  list.innerHTML = mcpServerConfigs
    .map((config, index) => {
      const target =
        config.transport.kind === "stdio"
          ? config.transport.config.program
          : config.transport.config.url;
      const credentialReferences =
        config.transport.kind === "stdio"
          ? Object.values(config.transport.config.environment)
          : config.transport.config.headers.map((header) => header.value);
      const references = [
        ...new Set(
          credentialReferences
            .filter((value) => value.kind === "credential_reference")
            .map((value) => value.value),
        ),
      ];
      const credentialButtons = references
        .map(
          (reference) =>
            `<button class="text-btn" style="margin-top:8px" type="button" data-native-action="delete-mcp-credential" data-mcp-index="${index}" data-mcp-reference="${escapeHtml(reference)}">删除凭据 ${escapeHtml(reference)}</button>`,
        )
        .join("");
      return `<article class="provider-card"><header><div><strong>${escapeHtml(config.name)}</strong><small>${escapeHtml(config.id)} · ${escapeHtml(config.transport.kind)} · ${escapeHtml(target)}</small><small>Timeout ${config.timeoutMs} ms · 密钥值不存数据库且不回显</small></div><span class="status ${config.enabled ? "complete" : "waiting"}">${config.enabled ? "Enabled" : "Disabled"}</span></header><div class="row" style="margin-top:10px"><button class="ghost-btn" type="button" data-native-action="edit-mcp" data-mcp-index="${index}">载入编辑</button><button class="ghost-btn" type="button" data-native-action="test-mcp" data-mcp-index="${index}">审批并测试</button><button class="text-btn" type="button" data-native-action="delete-mcp" data-mcp-index="${index}">删除配置</button></div>${credentialButtons}<div id="mcpStatus${index}"></div></article>`;
    })
    .join("");
}

async function loadExtensionsWorkspace(): Promise<void> {
  if (!isTauri()) {
    renderSkillCatalog();
    renderMcpServerList();
    setSettingsStatus(
      "extensionActionStatus",
      "浏览器预览不会扫描磁盘；请在 LunaScope 原生应用中验证。",
    );
    await loadImportState();
    return;
  }
  try {
    setSettingsStatus("extensionActionStatus", "正在扫描 Skill 摘要与 MCP 元数据…");
    const [directories, skills, servers] = await Promise.all([
      invoke<ExtensionDirectories>("extension_directories"),
      invoke<SkillSummary[]>("discover_skills"),
      invoke<McpServerConfig[]>("list_mcp_server_configs"),
    ]);
    extensionDirectories = directories;
    skillSummaries = skills;
    mcpServerConfigs = servers;
    const panel = document.querySelector<HTMLElement>(
      "#nativeIntegrationsPanel, #nativeSettingsPanel",
    );
    if (panel && panel.querySelector("#skillCatalogList")) {
      const directory = panel.querySelector<HTMLElement>(".routing-review");
      if (directory) {
        directory.textContent = `ROOT · ${directories.root}\nSYSTEM SKILLS · ${directories.systemSkills}\nUSER SKILLS · ${directories.userSkills}\nTOOLS · ${directories.tools}\nMCP · ${directories.mcp}`;
      }
    }
    renderSkillCatalog();
    renderMcpServerList();
    setSettingsStatus(
      "extensionActionStatus",
      `已发现 ${skills.length} 个 Skill 摘要与 ${servers.length} 个 MCP Server；未加载任何 Skill 正文。`,
      "success",
    );
    await loadImportState();
  } catch (error) {
    setSettingsStatus(
      "extensionActionStatus",
      `扩展目录读取失败：${errorMessage(error)}`,
      "error",
    );
  }
}

function readMcpServerForm(): {
  config: McpServerConfig;
  credential: { referenceId: string; secret: string } | null;
} {
  const value = (id: string): string => {
    const input = document.querySelector<
      HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
    >(`#${id}`);
    if (!input) throw new Error(`missing MCP field: ${id}`);
    return input.value.trim();
  };
  const kind = value("mcpTransportKind");
  const transport: McpServerConfig["transport"] =
    kind === "stdio"
      ? {
          kind: "stdio",
          config: {
            program: value("mcpTarget"),
            args: JSON.parse(value("mcpArgs")),
            cwd: value("mcpCwd") || null,
            environment: JSON.parse(value("mcpEnvironment")),
          },
        }
      : {
          kind: "streamable_http",
          config: {
            url: value("mcpTarget"),
            headers: JSON.parse(value("mcpHeaders")),
          },
        };
  const referenceId = value("mcpCredentialReference");
  const secret = value("mcpCredentialSecret");
  return {
    config: {
      id: value("mcpServerId"),
      name: value("mcpServerName"),
      transport,
      timeoutMs: Number(value("mcpTimeout")),
      enabled:
        document.querySelector<HTMLInputElement>("#mcpEnabled")?.checked ??
        true,
    },
    credential:
      referenceId && secret ? { referenceId, secret } : null,
  };
}

function populateMcpServerForm(config: McpServerConfig): void {
  const values: Record<string, string> = {
    mcpServerId: config.id,
    mcpServerName: config.name,
    mcpTransportKind: config.transport.kind,
    mcpTimeout: String(config.timeoutMs),
    mcpTarget:
      config.transport.kind === "stdio"
        ? config.transport.config.program
        : config.transport.config.url,
    mcpArgs:
      config.transport.kind === "stdio"
        ? JSON.stringify(config.transport.config.args, null, 2)
        : "[]",
    mcpCwd:
      config.transport.kind === "stdio"
        ? (config.transport.config.cwd ?? "")
        : "",
    mcpEnvironment:
      config.transport.kind === "stdio"
        ? JSON.stringify(config.transport.config.environment, null, 2)
        : "{}",
    mcpHeaders:
      config.transport.kind === "streamable_http"
        ? JSON.stringify(config.transport.config.headers, null, 2)
        : "[]",
  };
  for (const [id, next] of Object.entries(values)) {
    const element = document.querySelector<
      HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
    >(`#${id}`);
    if (element) element.value = next;
  }
  const enabled = document.querySelector<HTMLInputElement>("#mcpEnabled");
  if (enabled) enabled.checked = config.enabled;
  const secret =
    document.querySelector<HTMLInputElement>("#mcpCredentialSecret");
  if (secret) secret.value = "";
  setSettingsStatus(
    "mcpActionStatus",
    `已载入 ${config.name}；Secret 保持为空。`,
  );
}

function renderLoadedSkill(loaded: LoadedSkill): void {
  const detail = document.querySelector<HTMLElement>("#loadedSkillDetail");
  if (!detail) return;
  detail.innerHTML = `<article class="provider-card" style="margin-top:12px"><header><div><strong>Loaded on demand · ${escapeHtml(loaded.summary.name)}</strong><small>${escapeHtml(loaded.summary.compatibility)} · SHA-256 ${escapeHtml(loaded.summary.contentSha256)}</small><small>Supporting files: ${escapeHtml(loaded.supportingFiles.join(", ") || "none")}</small></div></header><pre class="diff" style="max-height:320px;overflow:auto;margin-top:10px">${escapeHtml(loaded.instructions)}</pre></article>`;
}

function renderMcpStatus(index: number, status: McpServerStatus): void {
  const element = document.querySelector<HTMLElement>(`#mcpStatus${index}`);
  if (!element) return;
  mcpServerStatuses.set(status.serverConfigId, status);
  const toolButtons = status.tools
    .map(
      (tool, toolIndex) =>
        `<button class="ghost-btn" type="button" data-native-action="invoke-mcp" data-mcp-index="${index}" data-mcp-tool-index="${toolIndex}">${escapeHtml(tool.name)}</button>`,
    )
    .join("");
  element.innerHTML = `<div class="routing-review" style="margin-top:10px">${escapeHtml(status.serverName ?? status.serverConfigId)} ${escapeHtml(status.serverVersion ?? "")}\nTools (${status.tools.length}): ${escapeHtml(status.tools.map((tool) => tool.name).join(", ") || "none")}</div>${status.tools.length ? `<div class="field" style="margin-top:8px"><label for="mcpInvokeArgs${index}">Tool arguments（JSON object）</label><textarea class="textarea" id="mcpInvokeArgs${index}" spellcheck="false" style="min-height:58px">{}</textarea></div><div class="row">${toolButtons}</div><div id="mcpInvokeResult${index}"></div>` : ""}`;
}

function renderToolRoutingDecision(decision: ToolRoutingDecision): void {
  const result = document.querySelector<HTMLElement>("#toolRoutingResult");
  if (!result) return;
  const selected =
    decision.selectedTools.map((tool) => `✓ ${tool.id}`).join("\n") || "none";
  const approvals =
    decision.approvalRequests
      .map(
        (request) =>
          `? ${request.permission} · ${request.context.toolId ?? request.action}`,
      )
      .join("\n") || "none";
  const rejected =
    decision.rejections
      .map((item) => `× ${item.toolId} · ${item.reason}`)
      .join("\n") || "none";
  result.innerHTML = `<div class="routing-review">${escapeHtml(decision.rationale)}\n\nAllowed\n${escapeHtml(selected)}\n\nApproval required\n${escapeHtml(approvals)}\n\nRejected\n${escapeHtml(rejected)}</div>`;
}

function showSettingsPage(page: string, button: HTMLButtonElement): void {
  history.replaceState(null, "", `#/settings/${page}`);
  applyUiLanguage();
  document
    .querySelectorAll<HTMLButtonElement>("[data-settings-page]")
    .forEach((item) => item.classList.toggle("active", item === button));
  const panel = document.querySelector<HTMLElement>("#nativeSettingsPanel");
  if (!panel) return;
  pendingRoutingDraft = null;
  if (page === "providers") {
    panel.innerHTML = providerSettingsMarkup();
    void loadNativeProviderSettings();
  } else if (page === "skills-mcp") {
    panel.innerHTML = skillsMcpMarkup();
    void loadExtensionsWorkspace();
  } else if (page === "general") {
    panel.innerHTML = generalSettingsMarkup();
  } else if (page === "projects") {
    panel.innerHTML = projectSettingsMarkup();
  } else if (page === "ultranote") {
    panel.innerHTML = ultraNoteSettingsMarkup();
  } else {
    panel.innerHTML =
      '<div class="pane-head" style="padding-inline:0"><strong>Settings</strong></div><span class="settings-help">此设置域将在对应运行时里程碑接入。</span>';
  }
}

function hydrateSettingsRoute(): void {
  if (!location.hash.startsWith("#/settings/")) return;
  const panel = document.querySelector<HTMLElement>("#nativeSettingsPanel");
  if (!panel || panel.dataset.nativeHydrated === "true") return;
  panel.dataset.nativeHydrated = "true";
  applyUiLanguage();
  const page = location.hash.slice("#/settings/".length) || "general";
  const button =
    document.querySelector<HTMLButtonElement>(
      `[data-settings-page="${CSS.escape(page)}"]`,
    ) ??
    document.querySelector<HTMLButtonElement>(
      '[data-settings-page="general"]',
    );
  if (button) showSettingsPage(button.dataset.settingsPage ?? "general", button);
}

async function invokeWithAllowOnce<T>(
  command: string,
  args: Record<string, unknown>,
  confirmation: string,
): Promise<T> {
  try {
    return await invoke<T>(command, { ...args, allowOnce: false });
  } catch (error) {
    const message = errorMessage(error);
    if (!message.includes("APPROVAL_REQUIRED")) throw error;
    const automatic =
      bypassMode ||
      computerAccessMode === "self_approve" ||
      computerAccessMode === "full_access";
    if (!automatic && !window.confirm(`${confirmation}\n\n${message}`)) {
      throw new Error("用户未授予本次操作权限");
    }
    return invoke<T>(command, { ...args, allowOnce: true });
  }
}

async function handleNativeSettingsAction(
  button: HTMLButtonElement,
): Promise<void> {
  const action = button.dataset.nativeAction;
  if (!action) return;
  if (action === "project-open-manager") {
    await openProjectManager(false);
    return;
  }
  if (
    action === "save-general-preferences" ||
    action === "save-ultranote-preferences"
  ) {
    const language =
      (document.querySelector<HTMLSelectElement>("#uiLanguage")
        ?.value as UiLanguage | undefined) ?? userPreferences.language;
    const noteSpec =
      document.querySelector<HTMLTextAreaElement>("#ultraNoteSpec")?.value ??
      userPreferences.ultranoteNoteSpec;
    const modelReplyLanguage =
      (document.querySelector<HTMLSelectElement>("#modelReplyLanguage")
        ?.value as ModelReplyLanguage | undefined) ??
      userPreferences.modelReplyLanguage;
    const next: UserPreferences = {
      language,
      modelReplyLanguage,
      ultranoteNoteSpec: noteSpec.trim(),
    };
    try {
      userPreferences = isTauri()
        ? await invoke<UserPreferences>("save_user_preferences", {
            preferences: next,
          })
        : next;
      applyUiLanguage();
      const statusId =
        action === "save-general-preferences"
          ? "generalPreferencesStatus"
          : "ultraNotePreferencesStatus";
      if (action === "save-general-preferences") {
        showSettingsPage(
          "general",
          document.querySelector<HTMLButtonElement>(
            '[data-settings-page="general"]',
          ) ?? button,
        );
      }
      setSettingsStatus(
        statusId,
        tr("偏好已保存。", "Preferences saved."),
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        action === "save-general-preferences"
          ? "generalPreferencesStatus"
          : "ultraNotePreferencesStatus",
        errorMessage(error),
        "error",
      );
    }
    return;
  }
  if (action === "native-orchestration-mode") {
    const mode = button.dataset.nativeMode;
    if (
      mode === "blueprint" ||
      mode === "runtime" ||
      mode === "queue" ||
      mode === "handoffs"
    ) {
      nativeOrchestrationMode = mode;
      applyNativeOrchestrationMode();
      document
        .querySelector<HTMLElement>("#nativeOrchestrationScroll")
        ?.scrollTo({ top: 0, behavior: "smooth" });
    }
    return;
  }
  if (action === "show-orchestration-add-worker") {
    nativeOrchestrationMode = "blueprint";
    applyNativeOrchestrationMode();
    document
      .querySelector<HTMLElement>("#nativeOrchestrationAddPanel")
      ?.scrollIntoView({ behavior: "smooth", block: "center" });
    window.setTimeout(
      () =>
        document
          .querySelector<HTMLInputElement>("#nativeOrchestrationNewRole")
          ?.focus(),
      250,
    );
    return;
  }
  if (action === "native-orchestration-layout") {
    renderNativeOrchestration();
    setSettingsStatus(
      "nativeOrchestrationStatus",
      "Auto Layout reapplied from the durable dependency graph.",
      "success",
    );
    return;
  }
  if (action === "native-orchestration-fit") {
    const graph = document.querySelector<HTMLElement>(".native-graph-scroll");
    graph?.scrollTo({ left: 0, top: 0, behavior: "smooth" });
    setSettingsStatus(
      "nativeOrchestrationStatus",
      "Blueprint returned to its fitted origin.",
      "success",
    );
    return;
  }
  if (action === "native-orchestration-focus") {
    await document
      .querySelector<HTMLElement>('[data-od-id="orchestration-workspace"]')
      ?.requestFullscreen?.();
    return;
  }
  if (action === "inspect-orchestration-node") {
    const kind = button.dataset.orchestrationNode;
    if (kind === "orchestrator" || kind === "synthesis") {
      openNativeInspector(kind);
    }
    return;
  }
  if (action === "focus-orchestration-worker") {
    const index = Number(button.dataset.orchestrationWorkerIndex);
    openNativeInspector("worker", index);
    return;
  }
  if (action === "native-inspector-tab") {
    nativeInspectorTab = button.dataset.inspectorTab ?? "overview";
    renderNativeInspector();
    return;
  }
  if (action === "draft-native-orchestration") {
    if (!isTauri()) {
      setSettingsStatus(
        "nativeOrchestrationStatus",
        "Browser preview cannot create durable Worker graphs.",
        "error",
      );
      return;
    }
    const objective =
      document
        .querySelector<HTMLTextAreaElement>("#nativeOrchestrationObjective")
        ?.value.trim() ?? "";
    const providerConfigId =
      document.querySelector<HTMLSelectElement>("#nativeOrchestrationProvider")
        ?.value ?? "";
    const modelId =
      document
        .querySelector<HTMLInputElement>("#nativeOrchestrationModel")
        ?.value.trim() ?? "";
    const domainPackId =
      (document.querySelector<HTMLSelectElement>(
        "#nativeOrchestrationDomain",
      )?.value as DomainPackId | "") || null;
    try {
      button.disabled = true;
      setSettingsStatus(
        "nativeOrchestrationStatus",
        "Analyzing delegation benefit and committing the Worker graph…",
      );
      nativeOrchestrationSession = await invoke<OrchestrationSession>(
        "draft_native_orchestration",
        {
          objective,
          domainPackId,
          workspaceRoot: activeWorkspaceRoot || null,
          projectId: activeProjectId,
          threadId: activeConversationThreadId,
          attachmentIds: [],
          onProgress: createOrchestrationProgressChannel(
            activeConversationRunId ?? `planning-${Date.now()}`,
          ),
          allowOnce: true,
        },
      );
      nativeOrchestrationResult = null;
      renderNativeOrchestration();
      renderNativeOrchestrationResult();
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `${nativeOrchestrationSession.plan.decision.kind} committed as graph v${nativeOrchestrationSession.plan.version}; ${nativeOrchestrationSession.plan.workers.length} complete WorkerSpec prompts are editable before execution.`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `Draft failed: ${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "patch-orchestration-worker") {
    const session = nativeOrchestrationSession;
    const index = Number(button.dataset.orchestrationWorkerIndex);
    const worker = session?.plan.workers[index];
    if (!session || !worker) return;
    const value = (id: string, fallback: string): string =>
      document.querySelector<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>(
        `#${id}`,
      )?.value.trim() ?? fallback;
    const csv = (id: string, fallback: string[]): string[] =>
      value(id, fallback.join(","))
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean);
    const lockPrompt =
      document.querySelector<HTMLInputElement>(`#orchLockPrompt${index}`)
        ?.checked ?? false;
    const lockRole =
      document.querySelector<HTMLInputElement>(`#orchLockRole${index}`)
        ?.checked ?? false;
    const lockTags =
      document.querySelector<HTMLInputElement>(`#orchLockTags${index}`)
        ?.checked ?? false;
    const nextRole = value(`orchRole${index}`, worker.role);
    const roleChanged = nextRole !== worker.role;
    const desiredLocks = [
      ...(lockPrompt ? (["prompt"] as const) : []),
      ...(lockRole ? (["role"] as const) : []),
      ...(lockTags ? (["tags"] as const) : []),
    ];
    const editableLocks = ["prompt", "role", "tags"] as const;
    const patch: OrchestrationPatch = {
      patchId: `patch-${crypto.randomUUID()}`,
      baseVersion: session.plan.version,
      applyMode:
        (document.querySelector<HTMLSelectElement>(`#orchPatchMode${index}`)
          ?.value ?? "apply_now") as OrchestrationPatch["applyMode"],
      reason: "User edited Worker fields in the native Inspector.",
      operations: [
        {
          kind: "update_worker",
          data: {
            patch: {
              workerId: worker.workerId,
              role: nextRole,
              tags: csv(`orchTags${index}`, worker.tags),
              objective: null,
              task: value(`orchTask${index}`, worker.task),
              prompt:
                document.querySelector<HTMLTextAreaElement>(
                  `#orchPrompt${index}`,
                )?.value ?? worker.prompt,
              inputContext: null,
              expectedOutput: roleChanged
                ? roleAssignment(nextRole, session.plan.objective)
                    .expectedOutput
                : null,
              outputSchema: roleChanged
                ? nextRole === "verifier"
                  ? verifierOutputSchema()
                  : evidenceOutputSchema()
                : null,
              completionCriteria: null,
              model: {
                provider: value(`orchProvider${index}`, worker.model.provider),
                model: value(`orchModel${index}`, worker.model.model),
                reason:
                  "User-selected and locked to the allowed Worker Model Pool.",
                fallback: false,
              },
              skills: csv(`orchSkills${index}`, worker.skills),
              tools: csv(`orchTools${index}`, worker.tools),
              permissions: csv(`orchPermissions${index}`, worker.permissions),
              budget: null,
              dependencies: csv(`orchDependencies${index}`, worker.dependencies),
              timeoutMs: null,
              retryPolicy: null,
              checkpointPolicy: null,
              writeScopes: csv(`orchWriteScopes${index}`, worker.writeScopes),
              parentWorkerId: null,
              lockFields: desiredLocks.filter(
                (field) => !worker.lockedFields.includes(field),
              ),
              unlockFields: editableLocks.filter(
                (field) =>
                  worker.lockedFields.includes(field) &&
                  !desiredLocks.includes(field),
              ),
            },
          },
        },
      ],
    };
    try {
      button.disabled = true;
      nativeOrchestrationSession = await invoke<OrchestrationSession>(
        "patch_native_orchestration",
        { runId: session.runId, patch },
      );
      renderNativeOrchestration();
      renderNativeInspector();
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `User Patch committed as graph v${nativeOrchestrationSession.plan.version}; overrides will not be silently reverted.`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `Patch rejected: ${errorMessage(error)}`,
        "error",
      );
      button.disabled = false;
    }
    return;
  }
  if (action === "remove-orchestration-worker") {
    const session = nativeOrchestrationSession;
    const worker =
      session?.plan.workers[
        Number(button.dataset.orchestrationWorkerIndex)
      ];
    if (!session || !worker) return;
    const patch: OrchestrationPatch = {
      patchId: `patch-${crypto.randomUUID()}`,
      baseVersion: session.plan.version,
      applyMode: "apply_now",
      reason: "User removed a Draft Worker.",
      operations: [
        {
          kind: "remove_worker",
          data: { worker_id: worker.workerId },
        },
      ],
    };
    try {
      nativeOrchestrationSession = await invoke<OrchestrationSession>(
        "patch_native_orchestration",
        { runId: session.runId, patch },
      );
      renderNativeOrchestration();
      closeNativeInspector();
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `Worker removed; graph v${nativeOrchestrationSession.plan.version} passed dependency validation.`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `Remove rejected: ${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "add-orchestration-worker") {
    const session = nativeOrchestrationSession;
    const source = session?.plan.workers[0];
    if (!session || !source) return;
    const provider =
      document.querySelector<HTMLSelectElement>("#nativeOrchestrationProvider")
        ?.value ?? source.model.provider;
    const model =
      document
        .querySelector<HTMLInputElement>("#nativeOrchestrationModel")
        ?.value.trim() || source.model.model;
    const role =
      document
        .querySelector<HTMLInputElement>("#nativeOrchestrationNewRole")
        ?.value.trim() || "reviewer";
    const requestedTags =
      document
        .querySelector<HTMLInputElement>("#nativeOrchestrationNewTags")
        ?.value.split(",")
        .map((tag) => tag.trim())
        .filter(Boolean) ?? [];
    const assignment = roleAssignment(role, session.plan.objective);
    const spec: WorkerSpec = structuredClone(source);
    spec.workerId = `worker-user-${crypto.randomUUID()}`;
    spec.role = role;
    spec.tags = requestedTags.length ? requestedTags : tagsForRole(role);
    spec.task = assignment.task;
    spec.prompt = assignment.prompt;
    spec.expectedOutput = assignment.expectedOutput;
    spec.outputSchema =
      role === "verifier" ? verifierOutputSchema() : evidenceOutputSchema();
    spec.model = {
      provider,
      model,
      reason: "User added this Worker from the allowed model pool.",
      fallback: false,
    };
    spec.tools = assignment.readOnly
      ? ["filesystem.read"]
      : ["filesystem.read", "filesystem.patch", "process.run"];
    spec.permissions = assignment.readOnly
      ? ["filesystem_read"]
      : ["filesystem_read", "filesystem_write", "process_spawn"];
    spec.dependencies = [];
    spec.writeScopes = assignment.readOnly
      ? []
      : [
          role === "frontend"
            ? "apps/desktop"
            : role === "backend"
              ? "crates"
              : role === "documentation"
                ? "docs"
                : "workspace-assigned-files",
        ];
    spec.lockedFields = [];
    spec.parentWorkerId = null;
    spec.depth = 0;
    const patch: OrchestrationPatch = {
      patchId: `patch-${crypto.randomUUID()}`,
      baseVersion: session.plan.version,
      applyMode: "apply_now",
      reason: `User added and labelled a ${role} Worker.`,
      operations: [{ kind: "add_worker", data: { spec } }],
    };
    try {
      nativeOrchestrationSession = await invoke<OrchestrationSession>(
        "patch_native_orchestration",
        { runId: session.runId, patch },
      );
      renderNativeOrchestration();
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `${role} Worker added; graph v${nativeOrchestrationSession.plan.version} and labels are durable.`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `Add Worker rejected: ${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "cancel-native-orchestration") {
    try {
      const cancelled = await invoke<boolean>("cancel_native_orchestration");
      setSettingsStatus(
        "nativeOrchestrationStatus",
        cancelled
          ? "Cancellation requested for every active Worker."
          : "No native orchestration is active.",
        cancelled ? "success" : "idle",
      );
    } catch (error) {
      setSettingsStatus(
        "nativeOrchestrationStatus",
        `Cancellation failed: ${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "refresh-providers") {
    await loadNativeProviderSettings();
    return;
  }
  if (action === "new-provider") {
    resetProviderForm();
    return;
  }
  if (action === "refresh-extensions") {
    await loadExtensionsWorkspace();
    return;
  }
  if (action === "preview-github-import") {
    const url =
      document.querySelector<HTMLInputElement>("#githubImportUrl")?.value.trim() ??
      "";
    try {
      button.disabled = true;
      setSettingsStatus(
        "githubImportStatus",
        "Waiting for approval to transfer Git objects into quarantine…",
      );
      githubImportPreview = await invokeWithAllowOnce<GithubImportPreview>(
        "preview_github_import",
        { workspaceRoot: extensionDirectories.userSkills, url },
        "Allow trusted Git to contact github.com and download objects into quarantine? No repository content will be checked out or executed.",
      );
      renderGithubImportPreview();
      renderInstalledImports();
      setSettingsStatus(
        "githubImportStatus",
        `Pinned ${githubImportPreview.commitSha}; inspected ${githubImportPreview.files.length} files and found ${githubImportPreview.components.length} selectable components.`,
        githubImportPreview.blocked ? "error" : "success",
      );
    } catch (error) {
      setSettingsStatus(
        "githubImportStatus",
        `Quarantine inspection failed: ${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "install-github-import") {
    if (!githubImportPreview) return;
    const componentIds = Array.from(
      document.querySelectorAll<HTMLInputElement>(
        "[data-import-component-id]:checked",
      ),
    ).map((input) => input.dataset.importComponentId ?? "");
    if (!componentIds.length) {
      setSettingsStatus(
        "githubImportStatus",
        "Select at least one inspected component before installation.",
        "error",
      );
      return;
    }
    try {
      button.disabled = true;
      const installed = await invokeWithAllowOnce<InstalledImportVersion>(
        "install_github_import",
        {
          request: {
            importId: githubImportPreview.importId,
            workspaceRoot: extensionDirectories.userSkills,
            componentIds,
          },
        },
        `Install ${componentIds.length} selected component(s) from pinned commit ${githubImportPreview.commitSha}? Files remain inert; no npm, bun, pip, cargo, postinstall, or shell command will run.`,
      );
      installedImportVersions = await invoke<InstalledImportVersion[]>(
        "list_installed_imports",
      );
      skillSummaries = await invoke<SkillSummary[]>("discover_skills");
      renderSkillCatalog();
      renderInstalledImports();
      setSettingsStatus(
        "githubImportStatus",
        `Installed pinned version ${installed.versionId} at ${installed.installedPath}.`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "githubImportStatus",
        `Installation failed: ${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "refresh-imports") {
    await loadImportState();
    return;
  }
  if (action === "rollback-import") {
    const version =
      installedImportVersions[Number(button.dataset.importVersionIndex)];
    if (!version) return;
    try {
      button.disabled = true;
      const restored = await invokeWithAllowOnce<InstalledImportVersion>(
        "rollback_github_import",
        {
          workspaceRoot: activeWorkspaceRoot,
          installationId: version.installationId,
          versionId: version.versionId,
        },
        `Restore pinned commit ${version.commitSha} from the retained local snapshot?`,
      );
      installedImportVersions = await invoke<InstalledImportVersion[]>(
        "list_installed_imports",
      );
      renderInstalledImports();
      setSettingsStatus(
        "githubImportStatus",
        `Restored ${restored.commitSha} from version ${restored.versionId}.`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "githubImportStatus",
        `Rollback failed: ${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "compare-import") {
    const version =
      installedImportVersions[Number(button.dataset.importVersionIndex)];
    if (!version || !githubImportPreview) return;
    try {
      const comparison = await invoke<ImportComparison>(
        "compare_github_import",
        {
          installationId: version.installationId,
          candidateImportId: githubImportPreview.importId,
        },
      );
      renderImportComparison(comparison);
      setSettingsStatus(
        "githubImportStatus",
        "Compared the active immutable file-object map with the candidate pinned commit.",
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "githubImportStatus",
        `Comparison failed: ${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "load-skill") {
    const skill = skillSummaries[Number(button.dataset.skillIndex)];
    if (!skill) return;
    try {
      button.disabled = true;
      setSettingsStatus(
        "extensionActionStatus",
        `正在按需读取 ${skill.name} 正文…`,
      );
      const loaded = await invokeWithAllowOnce<LoadedSkill>(
        "load_skill",
        {
          catalogId: skill.catalogId,
        },
        `允许本次加载 ${skill.path} 的 Skill 指令？该操作不会执行脚本。`,
      );
      renderLoadedSkill(loaded);
      setSettingsStatus(
        "extensionActionStatus",
        `${skill.name} 已按需加载；兼容级别 ${skill.compatibility}，未执行任何扩展代码。`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "extensionActionStatus",
        `Skill 加载失败：${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "route-with-skill") {
    const skill = skillSummaries[Number(button.dataset.skillIndex)];
    const input = document.querySelector<HTMLInputElement>("#routeSkills");
    if (skill && input) {
      const current = input.value
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean);
      input.value = [...new Set([...current, skill.catalogId])].join(", ");
      input.scrollIntoView({ behavior: "smooth", block: "center" });
    }
    return;
  }
  if (action === "edit-mcp") {
    const config = mcpServerConfigs[Number(button.dataset.mcpIndex)];
    if (config) populateMcpServerForm(config);
    return;
  }
  if (action === "test-mcp") {
    const index = Number(button.dataset.mcpIndex);
    const config = mcpServerConfigs[index];
    if (!config) return;
    try {
      button.disabled = true;
      setSettingsStatus(
        "mcpActionStatus",
        `正在评估 ${config.name} 的 MCP / transport / secrets 权限…`,
      );
      const status = await invokeWithAllowOnce<McpServerStatus>(
        "test_mcp_server",
        {
          workspaceRoot: activeWorkspaceRoot,
          serverConfigId: config.id,
        },
        `允许本次连接 ${config.name} 并发现其工具列表？`,
      );
      renderMcpStatus(index, status);
      setSettingsStatus(
        "mcpActionStatus",
        `${config.name} 健康检查通过，发现 ${status.tools.length} 个工具。`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "mcpActionStatus",
        `MCP 健康检查失败：${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "delete-mcp") {
    const config = mcpServerConfigs[Number(button.dataset.mcpIndex)];
    if (
      !config ||
      !window.confirm(
        `删除 ${config.name} 的配置元数据？Windows Credential Manager 中的 Secret 不会被静默删除。`,
      )
    ) {
      return;
    }
    try {
      await invoke("delete_mcp_server_config", { configId: config.id });
      mcpServerConfigs = await invoke<McpServerConfig[]>(
        "list_mcp_server_configs",
      );
      renderMcpServerList();
      setSettingsStatus(
        "mcpActionStatus",
        `${config.name} 配置已删除；Secret 保留，需显式删除。`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "mcpActionStatus",
        `删除 MCP 配置失败：${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "invoke-mcp") {
    const index = Number(button.dataset.mcpIndex);
    const toolIndex = Number(button.dataset.mcpToolIndex);
    const config = mcpServerConfigs[index];
    const tool = config
      ? mcpServerStatuses.get(config.id)?.tools[toolIndex]
      : undefined;
    if (!config || !tool) return;
    try {
      const source =
        document.querySelector<HTMLTextAreaElement>(`#mcpInvokeArgs${index}`)
          ?.value ?? "{}";
      const argumentsValue = JSON.parse(source);
      button.disabled = true;
      const result = await invokeWithAllowOnce<McpInvocationResult>(
        "invoke_mcp_tool",
        {
          workspaceRoot: activeWorkspaceRoot,
          serverConfigId: config.id,
          toolName: tool.name,
          arguments: argumentsValue,
        },
        `允许本次调用 ${config.name} 的工具 ${tool.name}？`,
      );
      const output = document.querySelector<HTMLElement>(
        `#mcpInvokeResult${index}`,
      );
      if (output) {
        output.innerHTML = `<div class="routing-review" style="margin-top:8px">${escapeHtml(JSON.stringify(result.content, null, 2))}\n${result.isError ? "MCP tool reported error" : "Completed"} · ${result.durationMs} ms</div>`;
      }
      setSettingsStatus(
        "mcpActionStatus",
        `${tool.name} 调用完成；返回内容按不可信数据展示。`,
        result.isError ? "error" : "success",
      );
    } catch (error) {
      setSettingsStatus(
        "mcpActionStatus",
        `MCP 工具调用失败：${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "delete-mcp-credential") {
    const config = mcpServerConfigs[Number(button.dataset.mcpIndex)];
    const referenceId = button.dataset.mcpReference;
    if (
      !config ||
      !referenceId ||
      !window.confirm(
        `从 Windows Credential Manager 删除 ${config.name} 的凭据 ${referenceId}？配置引用将保留。`,
      )
    ) {
      return;
    }
    try {
      await invoke("delete_mcp_credential", {
        serverConfigId: config.id,
        referenceId,
      });
      setSettingsStatus(
        "mcpActionStatus",
        `${config.name} 的凭据 ${referenceId} 已显式删除。`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "mcpActionStatus",
        `删除 MCP 凭据失败：${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "route-tools") {
    try {
      const csv = (id: string): string[] =>
        (
          document.querySelector<HTMLInputElement>(`#${id}`)?.value ?? ""
        )
          .split(",")
          .map((item) => item.trim())
          .filter(Boolean);
      const request: ToolRoutingRequest = {
        role:
          document.querySelector<HTMLSelectElement>("#routeRole")?.value ??
          "builder",
        objective:
          document.querySelector<HTMLInputElement>("#routeObjective")?.value ??
          "",
        preferredToolIds: csv("routePreferredTools"),
        requiredSkillCatalogIds: csv("routeSkills"),
        maxTools: Number(
          document.querySelector<HTMLInputElement>("#routeMaxTools")?.value ??
            "3",
        ),
        permissionContext: {
          workspaceRoot: activeWorkspaceRoot,
          projectId: null,
          runId: null,
          workerId: null,
          toolId: null,
          targetPath: null,
          program: null,
          networkDomain: null,
          arguments: [],
        },
      };
      const decision = await invoke<ToolRoutingDecision>("route_tools", {
        workspaceRoot: activeWorkspaceRoot,
        request,
      });
      renderToolRoutingDecision(decision);
      setSettingsStatus(
        "toolRouteStatus",
        "路由已由原生 Policy Engine 评估；Ask 项尚未执行。",
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "toolRouteStatus",
        `路由评估失败：${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "edit-provider") {
    const config = providerConfigs[Number(button.dataset.providerIndex)];
    if (config) populateProviderForm(config);
    return;
  }
  if (action === "save-model-settings") {
    try {
      const settings = readModelSelectionSettings();
      await invoke("save_model_selection_settings", {
        scopeId: globalRoutingScope,
        settings,
      });
      modelSelectionSettings = settings;
      setSettingsStatus(
        "modelSettingsStatus",
        "Orchestration Model 与 Worker Model Pool 已分开持久化。",
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "modelSettingsStatus",
        `保存模型分工失败：${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "save-structured-routing") {
    try {
      const policy = readStructuredPolicy();
      await invoke("save_routing_policy", {
        scopeId: globalRoutingScope,
        policy,
      });
      routingPolicy = policy;
      renderRoutingPolicy();
      setSettingsStatus(
        "routingActionStatus",
        "结构化路由偏好已保存。",
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "routingActionStatus",
        `保存结构化偏好失败：${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "delete-credential") {
    const config = providerConfigs[Number(button.dataset.providerIndex)];
    if (!config) return;
    if (
      !window.confirm(
        `删除 ${config.displayName} 的 Windows Credential Manager 凭据？配置元数据将保留。`,
      )
    ) {
      return;
    }
    try {
      await invoke("delete_provider_credential", {
        reference: {
          id: config.credentialReferenceId,
          provider: config.providerType,
          label: config.displayName,
        },
      });
      setSettingsStatus(
        "providerActionStatus",
        `${config.displayName} 的凭据已删除。`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "providerActionStatus",
        `删除凭据失败：${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "test-provider") {
    try {
      const { config, model } = readProviderForm();
      if (!providerConfigs.some((item) => item.id === config.id)) {
        throw new Error("请先保存配置，再执行连接测试");
      }
      button.disabled = true;
      setSettingsStatus("providerActionStatus", "正在执行 30 秒内的真实流式连接测试…");
      const result = await invoke<{
        responseId: string | null;
        text: string;
        inputTokens: number | null;
        outputTokens: number | null;
      }>("test_provider_connection", {
        providerConfigId: config.id,
        model,
      });
      setSettingsStatus(
        "providerActionStatus",
        `连接成功 · ${result.responseId ?? "no response id"} · ${result.inputTokens ?? "?"}/${result.outputTokens ?? "?"} tokens · ${result.text}`,
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "providerActionStatus",
        `连接失败：${errorMessage(error)}`,
        "error",
      );
    } finally {
      button.disabled = false;
    }
    return;
  }
  if (action === "parse-routing") {
    const source =
      document.querySelector<HTMLTextAreaElement>("#routingPreference")?.value ??
      "";
    try {
      pendingRoutingDraft = await invokeWithAllowOnce<NaturalLanguageRoutingDraft>(
        "parse_model_preference",
        { source, basePolicy: routingPolicy },
        tr(
          "允许编排模型解释这段自然语言路由偏好吗？",
          "Allow the Orchestration model to interpret this routing preference?",
        ),
      );
      renderRoutingPolicy();
      const confirm = document.querySelector<HTMLButtonElement>(
        '[data-native-action="confirm-routing"]',
      );
      if (confirm) confirm.disabled = false;
      setSettingsStatus(
        "routingActionStatus",
        "草案已生成，尚未生效。请检查后确认保存。",
      );
    } catch (error) {
      setSettingsStatus(
        "routingActionStatus",
        `解析失败：${errorMessage(error)}`,
        "error",
      );
    }
    return;
  }
  if (action === "confirm-routing" && pendingRoutingDraft) {
    try {
      const confirmed = {
        ...pendingRoutingDraft,
        confirmed: true,
      };
      await invoke("save_routing_policy", {
        scopeId: globalRoutingScope,
        policy: confirmed.policy,
      });
      routingPolicy = confirmed.policy;
      pendingRoutingDraft = null;
      button.disabled = true;
      renderRoutingPolicy();
      setSettingsStatus(
        "routingActionStatus",
        "路由策略已确认并持久化。",
        "success",
      );
    } catch (error) {
      setSettingsStatus(
        "routingActionStatus",
        `保存失败：${errorMessage(error)}`,
        "error",
      );
    }
  }
}

function bindNativeSettings(): void {
  document.addEventListener(
    "click",
    (event) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      const runControl = target.closest<HTMLElement>(
        "[data-native-run-control]",
      );
      if (runControl) {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (runControl.dataset.nativeRunControl === "pause") {
          void setNativeRunPaused(!nativeOrchestrationPaused).catch((error) => {
            window.lunaScopeUi?.appendConversationEvent({
              type: "assistant_message",
              title: "LunaScope",
              summary: `${tr("运行控制失败：", "Run control failed: ")}${errorMessage(error)}`,
            });
          });
        } else if (runControl.dataset.nativeRunControl === "cancel") {
          nativeOrchestrationPaused = false;
          void invoke<boolean>("cancel_native_orchestration")
            .then(() => {
              window.lunaScopeUi?.setExecutionActivity({
                running: true,
                workerId: null,
                role: "LunaScope",
                state: tr("正在取消", "Cancelling"),
                detail: tr(
                  "正在停止当前 Agent；已完成的文件与持久上下文会保留",
                  "Stopping active Agents; completed files and durable context will be preserved",
                ),
              });
            })
            .catch((error) => {
              window.lunaScopeUi?.appendConversationEvent({
                type: "assistant_message",
                title: "LunaScope",
                summary: `${tr("取消失败：", "Cancellation failed: ")}${errorMessage(error)}`,
              });
            });
        }
        return;
      }
      const deleteConversationButton =
        target.closest<HTMLElement>("[data-delete-thread]");
      if (deleteConversationButton) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const threadId = deleteConversationButton.dataset.deleteThread ?? "";
        const title =
          deleteConversationButton
            .closest(".thread-row")
            ?.querySelector(".thread-main strong")?.textContent?.trim() ||
          tr("此对话", "this conversation");
        void deleteConversationById(threadId, title).catch((error) => {
          window.alert(
            `${tr("删除对话失败：", "Failed to delete conversation: ")}${errorMessage(error)}`,
          );
        });
        return;
      }
      const conversationButton =
        target.closest<HTMLElement>("button[data-thread]");
      if (conversationButton) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const nextThreadId = conversationButton.dataset.thread ?? "";
        if (
          nativeOrchestrationRunning &&
          nextThreadId !== activeConversationThreadId
        ) {
          window.alert(
            tr(
              "Agent 运行期间不能切换对话。请先等待完成或停止运行。",
              "You cannot switch conversations while Agents are running. Wait for completion or stop the run first.",
            ),
          );
          return;
        }
        const reference = window.lunaScopeUi?.activateConversation(
          nextThreadId,
        );
        applyActiveConversationReference(reference ?? null);
        return;
      }
      const retryButton = target.closest("[data-retry-conversation]");
      if (retryButton) {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!nativeOrchestrationRunning) {
          const eventId =
            retryButton.closest<HTMLElement>("[data-event-id]")?.dataset
              .eventId ?? "";
          const sourceRunId = eventId.startsWith("orchestration-run-")
            ? eventId.slice("orchestration-run-".length)
            : undefined;
          void retryFailedAgents(sourceRunId).catch((error) => {
            window.lunaScopeUi?.appendConversationEvent({
              type: "assistant_message",
              title: "LunaScope",
              summary: `${tr("重试失败：", "Retry failed: ")}${errorMessage(error)}`,
              retryConversation: true,
            });
          });
        }
        return;
      }
      const dismissRetry = target.closest<HTMLElement>("[data-dismiss-retry]");
      if (dismissRetry) {
        event.preventDefault();
        event.stopImmediatePropagation();
        dismissRetry.closest(".retry-actions")?.remove();
        return;
      }
      if (target.closest('button[data-special="integrations"]')) {
        queueMicrotask(hydrateIntegrationHub);
        return;
      }
      if (target.closest('button[data-view="orchestration"]')) {
        queueMicrotask(() => void hydrateOrchestrationHub());
        return;
      }
      if (target.closest('button[data-special="settings"]')) {
        queueMicrotask(() => {
          const general = document.querySelector<HTMLButtonElement>(
            '[data-settings-page="general"]',
          );
          if (general) showSettingsPage("general", general);
        });
        return;
      }
      if (target.closest("#newTask")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void (activeProjectId
          ? createNewConversation()
          : openProjectManager(true));
        return;
      }
      if (target.closest("#projectSwitch")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void openProjectManager(false);
        return;
      }
      const removeAttachment = target.closest<HTMLButtonElement>(
        '[data-native-action="remove-attachment"]',
      );
      if (removeAttachment) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const attachmentId = removeAttachment.dataset.attachmentId ?? "";
        pendingAttachments = pendingAttachments.filter(
          (item) => item.attachmentId !== attachmentId,
        );
        renderPendingAttachments();
        if (attachmentId && isTauri()) {
          void invoke("remove_imported_attachment", { attachmentId });
        }
        return;
      }
      if (target.closest('[data-composer="attach"]')) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void chooseAttachments().catch((error) => {
          window.lunaScopeUi?.appendConversationEvent({
            type: "assistant_message",
            title: "LunaScope",
            summary: `${tr("附件读取失败：", "Attachment import failed: ")}${errorMessage(error)}`,
          });
        });
        return;
      }
      if (target.closest('[data-composer="command"]')) {
        event.preventDefault();
        event.stopImmediatePropagation();
        showSlashMenu();
        return;
      }
      if (target.closest("#closeReview") && nativeInspector) {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeNativeInspector();
        return;
      }
      if (!target.closest("#send")) return;
      const input =
        document.querySelector<HTMLTextAreaElement>("#composerInput");
      if (!input) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const value = input.value.trim();
      if (!value) return;
      if (value.startsWith("/") && handleSlashCommand(value)) {
        input.value = "";
        return;
      }
      input.value = "";
      void submitConversation(value).catch((error) => {
        window.lunaScopeUi?.appendConversationEvent({
          id: "orchestration-thinking",
          type: "assistant_message",
          title: "LunaScope",
          summary: `${tr("任务未能完成：", "Task could not complete: ")}${errorMessage(error)}`,
        });
      });
    },
    true,
  );
  document.addEventListener(
    "keydown",
    (event) => {
      if (
        event.key !== "Enter" ||
        event.shiftKey ||
        !(event.target instanceof HTMLTextAreaElement) ||
        event.target.id !== "composerInput" ||
        !event.target.value.trim().startsWith("/")
      ) {
        return;
      }
      if (!handleSlashCommand(event.target.value)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      event.target.value = "";
    },
    true,
  );
  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const integrationsButton = target.closest<HTMLButtonElement>(
      'button[data-special="integrations"]',
    );
    if (integrationsButton) {
      queueMicrotask(hydrateIntegrationHub);
      return;
    }
    const orchestrationButton = target.closest<HTMLButtonElement>(
      'button[data-view="orchestration"]',
    );
    if (orchestrationButton) {
      queueMicrotask(() => void hydrateOrchestrationHub());
      return;
    }
    const settingsButton = target.closest<HTMLButtonElement>(
      'button[data-special="settings"]',
    );
    if (settingsButton) {
      queueMicrotask(() => {
        const general = document.querySelector<HTMLButtonElement>(
          '[data-settings-page="general"]',
        );
        if (general) showSettingsPage("general", general);
      });
      return;
    }
    const pageButton = target.closest<HTMLButtonElement>(
      "button[data-settings-page]",
    );
    if (pageButton) {
      showSettingsPage(pageButton.dataset.settingsPage ?? "", pageButton);
      return;
    }
    const actionButton = target.closest<HTMLButtonElement>(
      "button[data-native-action]",
    );
    if (actionButton) {
      void (async () => {
        if (await handleProjectAction(actionButton)) return;
        if (await handleUltraNoteAction(actionButton)) return;
        await handleNativeSettingsAction(actionButton);
      })();
    }
  }, true);
  document.addEventListener("change", (event) => {
    const target = event.target;
    if (
      target instanceof HTMLSelectElement &&
      target.id === "computerAccessMode"
    ) {
      computerAccessMode = target.value as ComputerAccessMode;
      saveAccessSettings();
      return;
    }
    if (target instanceof HTMLInputElement && target.id === "bypassMode") {
      bypassMode = target.checked;
      saveAccessSettings();
      return;
    }
    if (
      target instanceof HTMLInputElement &&
      target.name === "projectWorkspaceFolder"
    ) {
      projectWorkspacePath = target.value;
      return;
    }
    if (target instanceof HTMLSelectElement && target.id === "nativeProjectKind") {
      captureProjectDraftFields();
      projectDraftKind = target.value as ProjectKind;
      renderProjectDialog();
      return;
    }
    if (
      target instanceof HTMLInputElement &&
      target.id === "nativeOrchestrationNewRole"
    ) {
      const tags = document.querySelector<HTMLInputElement>(
        "#nativeOrchestrationNewTags",
      );
      if (tags) tags.value = tagsForRole(target.value.trim()).join(", ");
      return;
    }
    if (!(target instanceof HTMLSelectElement)) {
      return;
    }
    if (target.id === "nativeOrchestrationProvider") {
      const option = target.selectedOptions[0];
      const model = document.querySelector<HTMLInputElement>(
        "#nativeOrchestrationModel",
      );
      if (model) model.value = option?.dataset.model ?? "";
      return;
    }
    if (target.id === "nativeOrchestrationDomain") {
      const summary = document.querySelector<HTMLElement>(
        "#nativeDomainPackSummary",
      );
      const pack = domainPacks.find((item) => item.id === target.value);
      if (summary) {
        summary.textContent = pack
          ? `${pack.workflow.join(" → ")} · Evidence: ${pack.requiredEvidence.join(", ")}`
          : "Auto records its routing reason in the durable Plan.";
      }
      return;
    }
    if (target.id === "mcpTransportKind") {
      const targetInput =
        document.querySelector<HTMLInputElement>("#mcpTarget");
      if (targetInput) {
        targetInput.value =
          target.value === "stdio"
            ? "C:\\path\\to\\mcp-server.exe"
            : "https://example.com/mcp";
      }
      return;
    }
    if (target.id !== "providerType") return;
    const presets: Record<
      ProviderType,
      {
        protocol: ProviderProtocol;
        baseUrl: string;
        model: string;
        context: number | null;
        tools: boolean;
        vision: boolean;
        structured: boolean;
      }
    > = {
      open_ai: {
        protocol: "open_ai_responses",
        baseUrl: "https://api.openai.com",
        model: "gpt-5-mini",
        context: 128_000,
        tools: true,
        vision: true,
        structured: true,
      },
      anthropic: {
        protocol: "anthropic_messages",
        baseUrl: "https://api.anthropic.com",
        model: "claude-sonnet-4-20250514",
        context: 200_000,
        tools: true,
        vision: true,
        structured: false,
      },
      deep_seek: {
        protocol: "open_ai_chat_completions",
        baseUrl: "https://api.deepseek.com",
        model: "deepseek-v4-flash",
        context: 1_000_000,
        tools: true,
        vision: false,
        structured: true,
      },
      generic_open_ai_compatible: {
        protocol: "open_ai_chat_completions",
        baseUrl: "http://127.0.0.1:11434/v1",
        model: "local-model",
        context: null,
        tools: false,
        vision: false,
        structured: false,
      },
      generic_anthropic_compatible: {
        protocol: "anthropic_messages",
        baseUrl: "http://127.0.0.1:8080",
        model: "local-model",
        context: null,
        tools: false,
        vision: false,
        structured: false,
      },
    };
    const preset = presets[target.value as ProviderType];
    const protocol =
      document.querySelector<HTMLSelectElement>("#providerProtocol");
    const baseUrl = document.querySelector<HTMLInputElement>("#providerBaseUrl");
    const model = document.querySelector<HTMLInputElement>("#providerTestModel");
    const context =
      document.querySelector<HTMLInputElement>("#providerContextWindow");
    if (protocol) protocol.value = preset.protocol;
    if (baseUrl) baseUrl.value = preset.baseUrl;
    if (model) model.value = preset.model;
    if (context) context.value = preset.context?.toString() ?? "";
    const presetChecks: Array<[string, boolean]> = [
      ["providerSupportsTools", preset.tools],
      ["providerSupportsVision", preset.vision],
      ["providerSupportsStructured", preset.structured],
    ];
    for (const [id, checked] of presetChecks) {
      const input = document.querySelector<HTMLInputElement>(`#${id}`);
      if (input) input.checked = checked;
    }
  });
  document.addEventListener("submit", (event) => {
    const form = event.target;
    if (!(form instanceof HTMLFormElement)) {
      return;
    }
    if (form.id === "mcpServerForm") {
      event.preventDefault();
      void (async () => {
        try {
          const { config, credential } = readMcpServerForm();
          setSettingsStatus("mcpActionStatus", "正在验证并保存 MCP 配置…");
          await invoke("save_mcp_server_config", { config, credential });
          const secret =
            document.querySelector<HTMLInputElement>("#mcpCredentialSecret");
          if (secret) secret.value = "";
          mcpServerConfigs = await invoke<McpServerConfig[]>(
            "list_mcp_server_configs",
          );
          renderMcpServerList();
          setSettingsStatus(
            "mcpActionStatus",
            "MCP 配置已保存；Secret 输入框已清空，数据库仅保留引用。",
            "success",
          );
        } catch (error) {
          setSettingsStatus(
            "mcpActionStatus",
            `保存 MCP 配置失败：${errorMessage(error)}`,
            "error",
          );
        }
      })();
      return;
    }
    if (form.id !== "providerConfigForm") return;
    event.preventDefault();
    void (async () => {
      try {
        const { config, credential } = readProviderForm();
        setSettingsStatus("providerActionStatus", "正在保存本机配置…");
        await invoke("save_provider_config", { config, credential });
        const secret =
          document.querySelector<HTMLInputElement>("#providerCredential");
        if (secret) secret.value = "";
        setSettingsStatus(
          "providerActionStatus",
          "配置已保存；API Key 输入框已清空。",
          "success",
        );
        providerConfigs =
          await invoke<ProviderConfig[]>("list_provider_configs");
        renderProviderList();
      } catch (error) {
        setSettingsStatus(
          "providerActionStatus",
          `保存失败：${errorMessage(error)}`,
          "error",
        );
      }
      })();
  });
  const view = document.querySelector<HTMLElement>("#view");
  if (view) {
    new MutationObserver(() => {
      hydrateIntegrationHub();
      void hydrateOrchestrationHub();
      hydrateSettingsRoute();
    }).observe(view, {
      childList: true,
      subtree: true,
    });
  }
  window.addEventListener("hashchange", () => {
    if (location.hash.endsWith("/ultranote")) {
      queueMicrotask(() => void openUltraNoteWorkspace());
    }
    queueMicrotask(hydrateSettingsRoute);
  });
  hydrateIntegrationHub();
  void hydrateOrchestrationHub();
  hydrateSettingsRoute();
  if (location.hash.endsWith("/ultranote")) {
    queueMicrotask(() => void openUltraNoteWorkspace());
  }
}

async function bootNativeRuntime(): Promise<void> {
  bindNativeSettings();
  loadAccessSettings();
  applyAccessSettings();
  renderNativeRunControls();
  await loadUserPreferences();
  if (!isTauri()) {
    document.documentElement.dataset.runtimeAuthority = "browser-preview";
    return;
  }

  await listen<{ paths: string[] }>("tauri://drag-drop", (event) => {
    const paths = event.payload?.paths ?? [];
    if (!paths.length) return;
    void importAttachmentPaths(paths).catch((error) => {
      window.lunaScopeUi?.appendConversationEvent({
        type: "assistant_message",
        title: "LunaScope",
        summary: `${tr("拖放附件读取失败：", "Dropped attachment import failed: ")}${errorMessage(error)}`,
      });
    });
  });
  projects = await invoke<LunaProject[]>("list_projects");
  activeProjectId = projects[0]?.projectId ?? null;
  applyActiveConversationReference(
    window.lunaScopeUi?.activeConversation() ?? null,
    true,
  );
  syncActiveWorkspace();
  applyUiLanguage();
  await invoke<RuntimeSnapshot>("create_run", {
    request: {
      runId: bootstrapRunId,
      title: "LunaScope native runtime",
      initialPrompt: "Restore the durable desktop session",
    },
  });
  // UI authority begins with the ordered native channel, not a command
  // response or browser-local persistence.
  await subscribe(0);
}

void bootNativeRuntime().catch((error: unknown) => {
  document.documentElement.dataset.runtimeAuthority = "native-error";
  console.error("Failed to initialize native runtime", error);
  const status = document.querySelector<HTMLElement>("#statusText");
  if (status) {
    status.textContent = "NATIVE RUNTIME ERROR";
  }
});
