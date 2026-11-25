export type AgentMode = "ask" | "plan" | "autonomous";

export type ProviderId =
  | "openai"
  | "anthropic"
  | "gemini"
  | "openrouter"
  | "ollama"
  | "custom";

export interface DriveInfo {
  letter: string;
  mountPath: string;
  driveType: string;
  totalGb: number;
  freeGb: number;
}

export interface FileEntry {
  name: string;
  path: string;
  relPath: string;
  isDir: boolean;
  size: number;
  readonly: boolean;
}

export interface SearchMatch {
  file: string;
  relPath: string;
  line: number;
  column: number;
  preview: string;
}

export interface GitChangedFile {
  path: string;
  status: string;
  staged: boolean;
}

export interface GitCommitSummary {
  hash: string;
  subject: string;
  author: string;
  date: string;
}

export interface GitStatusPayload {
  isRepo: boolean;
  branch: string;
  branches: string[];
  files: GitChangedFile[];
  commits: GitCommitSummary[];
}

export interface AppSettings {
  activeProvider: ProviderId;
  activeModel: string;
  defaultMode: AgentMode;
  openaiApiKey: string;
  anthropicApiKey: string;
  geminiApiKey: string;
  openrouterApiKey: string;
  ollamaBaseUrl: string;
  customBaseUrl: string;
  customApiKey: string;
  requireShellApproval: boolean;
  requireDeleteConfirmation: boolean;
  restrictToWorkspace: boolean;
  maxAgentIterations: number;
  defaultShell: "powershell" | "cmd";
  editorFontSize: number;
}

export interface ToolExecutionRecord {
  id: string;
  sessionId: string;
  toolName: string;
  argsJson: string;
  resultSummary: string;
  status: "running" | "ok" | "error" | "denied";
  durationMs: number;
  diffText?: string;
  filePath?: string;
  createdAt: string;
}

export interface ChatMessage {
  id: string;
  sessionId: string;
  role: "user" | "assistant" | "system";
  content: string;
  thinking?: string;
  mode: AgentMode;
  provider?: string;
  model?: string;
  tools?: ToolExecutionRecord[];
  createdAt: string;
}

export interface SessionSummary {
  id: string;
  title: string;
  projectPath: string;
  mode: AgentMode;
  updatedAt: string;
  messageCount: number;
}

export interface SessionDetail {
  id: string;
  title: string;
  projectPath: string;
  mode: AgentMode;
  createdAt: string;
  updatedAt: string;
  messages: ChatMessage[];
}

export interface AppInitPayload {
  appVersion: string;
  platform: string;
  appDataDir: string;
  sqlitePath: string;
  currentProject: string;
  drives: DriveInfo[];
  files: FileEntry[];
  git: GitStatusPayload;
  settings: AppSettings;
  sessions: SessionSummary[];
  activeSession: SessionDetail;
  recentProjects: string[];
}

export interface PermissionRequestEvent {
  requestId: string;
  kind: "shell" | "delete_file" | "outside_workspace";
  title: string;
  detail: string;
  command?: string;
  targetPath?: string;
  cwd?: string;
}

export interface AgentStreamEvent {
  sessionId: string;
  kind:
    | "status"
    | "thinking"
    | "text"
    | "tool_start"
    | "tool_end"
    | "error"
    | "done";
  text?: string;
  statusText?: string;
  tool?: ToolExecutionRecord;
  session?: SessionDetail;
}

export interface OpenEditorTab {
  id: string;
  kind: "code" | "diff";
  path: string;
  relPath: string;
  title: string;
  language: string;
  content: string;
  savedContent: string;
  readonly: boolean;
  originalContent?: string;
}

export interface PtyTabSession {
  ptyId: string;
  title: string;
  shell: "powershell" | "cmd";
  cwd: string;
  alive: boolean;
}
