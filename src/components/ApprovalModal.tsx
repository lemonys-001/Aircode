import React from "react";
import { ShieldAlert, Check, X, Terminal, Trash2 } from "lucide-react";
import { PermissionRequestEvent } from "../types/desktop";

interface Props {
  request: PermissionRequestEvent | null;
  onDecision: (
    requestId: string,
    approved: boolean,
    rememberSession: boolean
  ) => void;
}

export const ApprovalModal: React.FC<Props> = ({ request, onDecision }) => {
  if (!request) return null;

  return (
    <div className="desktop-modal-backdrop">
      <div className="desktop-modal approval-dialog">
        <div className="modal-head">
          <div className="modal-title-row">
            <ShieldAlert size={18} className="warn-icon" />
            <h3>{request.title}</h3>
          </div>
        </div>

        <div className="modal-content">
          <p className="approval-desc">{request.detail}</p>

          {request.command && (
            <div className="approval-code-box">
              <div className="approval-box-label">
                <Terminal size={13} /> Shell Command ({request.cwd || "project"})
              </div>
              <pre>{request.command}</pre>
            </div>
          )}

          {request.targetPath && (
            <div className="approval-code-box danger-border">
              <div className="approval-box-label">
                <Trash2 size={13} /> Target File / Directory
              </div>
              <pre>{request.targetPath}</pre>
            </div>
          )}
        </div>

        <div className="modal-actions">
          <button
            className="glass-btn danger"
            onClick={() => onDecision(request.requestId, false, false)}
          >
            <X size={14} /> Deny Operation
          </button>
          {request.kind === "shell" && (
            <button
              className="glass-btn"
              onClick={() => onDecision(request.requestId, true, true)}
            >
              <Check size={14} /> Allow for This Session
            </button>
          )}
          <button
            className="glass-btn accent"
            onClick={() => onDecision(request.requestId, true, false)}
          >
            <Check size={14} /> Approve Once
          </button>
        </div>
      </div>
    </div>
  );
};
