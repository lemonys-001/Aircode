import React, { useMemo, useState } from "react";
import {
  FileCode2,
  Folder,
  FolderOpen,
  FilePlus2,
  FolderPlus,
  RefreshCw,
  Trash2,
  Search,
  ExternalLink,
  Lock,
} from "lucide-react";
import { FileEntry, SearchMatch } from "../types/desktop";

interface Props {
  projectRoot: string;
  files: FileEntry[];
  activeFilePath: string | null;
  watcherActive: boolean;
  onOpenFile: (entry: FileEntry, line?: number) => void;
  onRefresh: () => void;
  onCreateItem: (relPath: string, isDir: boolean) => void;
  onDeleteItem: (entry: FileEntry) => void;
  onRevealExplorer: (path: string) => void;
  onRunSearch: (query: string) => Promise<SearchMatch[]>;
}

interface TreeNode {
  name: string;
  relPath: string;
  fullPath: string;
  isDir: boolean;
  entry?: FileEntry;
  children: Map<string, TreeNode>;
}

export const FileExplorer: React.FC<Props> = ({
  projectRoot,
  files,
  activeFilePath,
  watcherActive,
  onOpenFile,
  onRefresh,
  onCreateItem,
  onDeleteItem,
  onRevealExplorer,
  onRunSearch,
}) => {
  const [subTab, setSubTab] = useState<"tree" | "search">("tree");
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [searchQuery, setSearchQuery] = useState("");
  const [searchHits, setSearchHits] = useState<SearchMatch[]>([]);
  const [searching, setSearching] = useState(false);

  const treeRoot = useMemo(() => {
    const root: TreeNode = {
      name: "",
      relPath: "",
      fullPath: projectRoot,
      isDir: true,
      children: new Map(),
    };
    const q = filter.trim().toLowerCase();
    for (const f of files) {
      if (q && !f.relPath.toLowerCase().includes(q)) continue;
      const parts = f.relPath.replace(/\\/g, "/").split("/").filter(Boolean);
      let curr = root;
      parts.forEach((part, idx) => {
        const isLast = idx === parts.length - 1;
        const subRel = parts.slice(0, idx + 1).join("/");
        if (!curr.children.has(part)) {
          curr.children.set(part, {
            name: part,
            relPath: subRel,
            fullPath: isLast ? f.path : `${projectRoot}\\${subRel.replace(/\//g, "\\")}`,
            isDir: isLast ? f.isDir : true,
            entry: isLast ? f : undefined,
            children: new Map(),
          });
        }
        curr = curr.children.get(part)!;
      });
    }
    return root;
  }, [files, filter, projectRoot]);

  const toggleDir = (rel: string) => {
    setExpanded((prev) => ({ ...prev, [rel]: !(prev[rel] ?? true) }));
  };

  const renderNode = (node: TreeNode, depth: number): React.ReactNode => {
    const sorted = Array.from(node.children.values()).sort((a, b) =>
      a.isDir === b.isDir ? a.name.localeCompare(b.name) : a.isDir ? -1 : 1
    );
    return sorted.map((child) => {
      const isOpen = expanded[child.relPath] ?? depth < 1;
      const isSelected = activeFilePath === child.fullPath;
      return (
        <React.Fragment key={child.relPath}>
          <div
            className={`explorer-row ${isSelected ? "selected" : ""}`}
            style={{ paddingLeft: `${10 + depth * 14}px` }}
            onClick={() => {
              if (child.isDir) {
                toggleDir(child.relPath);
              } else if (child.entry) {
                onOpenFile(child.entry);
              }
            }}
          >
            <div className="explorer-row-main">
              {child.isDir ? (
                isOpen ? (
                  <FolderOpen size={14} className="folder-icon" />
                ) : (
                  <Folder size={14} className="folder-icon" />
                )
              ) : (
                <FileCode2 size={14} className="file-icon" />
              )}
              <span className="explorer-name" title={child.fullPath}>
                {child.name}
              </span>
              {child.entry?.readonly && (
                <span title="Read-only Windows file attribute">
                  <Lock size={11} className="muted-icon" />
                </span>
              )}
            </div>
            <div className="explorer-row-actions">
              <button
                className="tiny-icon-btn"
                title="Reveal in Windows Explorer"
                onClick={(e) => {
                  e.stopPropagation();
                  onRevealExplorer(child.fullPath);
                }}
              >
                <ExternalLink size={12} />
              </button>
              {child.entry && (
                <button
                  className="tiny-icon-btn danger"
                  title="Delete from Disk"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDeleteItem(child.entry!);
                  }}
                >
                  <Trash2 size={12} />
                </button>
              )}
            </div>
          </div>
          {child.isDir && isOpen && renderNode(child, depth + 1)}
        </React.Fragment>
      );
    });
  };

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!searchQuery.trim()) return;
    setSearching(true);
    try {
      const hits = await onRunSearch(searchQuery.trim());
      setSearchHits(hits);
    } finally {
      setSearching(false);
    }
  };

  return (
    <div className="side-pane">
      <div className="pane-header">
        <div className="pane-tab-pills">
          <button
            className={subTab === "tree" ? "active" : ""}
            onClick={() => setSubTab("tree")}
          >
            Files
          </button>
          <button
            className={subTab === "search" ? "active" : ""}
            onClick={() => setSubTab("search")}
          >
            <Search size={12} /> Search
          </button>
        </div>

        <div className="pane-header-btns">
          <span
            className={`watcher-dot ${watcherActive ? "live" : ""}`}
            title="Real-time Windows ReadDirectoryChangesW Watcher Active"
          />
          <button
            className="tiny-icon-btn"
            title="New File"
            onClick={() => {
              const name = window.prompt("New relative file path (e.g. src/utils.ts):");
              if (name) onCreateItem(name, false);
            }}
          >
            <FilePlus2 size={14} />
          </button>
          <button
            className="tiny-icon-btn"
            title="New Folder"
            onClick={() => {
              const name = window.prompt("New relative directory path:");
              if (name) onCreateItem(name, true);
            }}
          >
            <FolderPlus size={14} />
          </button>
          <button
            className="tiny-icon-btn"
            title="Refresh Filesystem"
            onClick={onRefresh}
          >
            <RefreshCw size={14} />
          </button>
        </div>
      </div>

      {subTab === "tree" ? (
        <>
          <div className="pane-filter-box">
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter workspace files..."
            />
          </div>
          <div className="explorer-scroll">{renderNode(treeRoot, 0)}</div>
        </>
      ) : (
        <div className="search-pane-body">
          <form onSubmit={handleSearch} className="search-form">
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search symbol or text across project..."
            />
            <button type="submit" className="glass-btn small">
              {searching ? "..." : "Find"}
            </button>
          </form>
          <div className="explorer-scroll">
            {searchHits.map((m, i) => (
              <div
                key={`${m.file}:${m.line}:${i}`}
                className="search-hit-card"
                onClick={() =>
                  onOpenFile(
                    {
                      name: m.relPath.split("/").pop() || m.relPath,
                      path: m.file,
                      relPath: m.relPath,
                      isDir: false,
                      size: 0,
                      readonly: false,
                    },
                    m.line
                  )
                }
              >
                <div className="search-hit-file">
                  {m.relPath}:{m.line}
                </div>
                <div className="search-hit-preview">{m.preview}</div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};
