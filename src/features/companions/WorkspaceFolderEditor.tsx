import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";

import styles from "./WorkspaceFolderEditor.module.css";

const MAX_LABEL_LENGTH = 80;

export interface WorkspaceFolderDraft {
  /** UI-only key. Existing rows use their persisted id. */
  key: string;
  id?: string;
  label: string;
  directory: string;
}

interface WorkspaceFolderEditorProps {
  value: WorkspaceFolderDraft[];
  disabled?: boolean;
  onChange: (workspaces: WorkspaceFolderDraft[]) => void;
  onError: (error: unknown) => void;
}

function folderBasename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? "Workspace";
}

function availableLabel(path: string, workspaces: WorkspaceFolderDraft[]): string {
  const rawBase = folderBasename(path).trim() || "Workspace";
  const base = Array.from(rawBase).slice(0, MAX_LABEL_LENGTH).join("");
  const used = new Set(
    workspaces.map((workspace) => workspace.label.trim().toLowerCase()),
  );
  if (!used.has(base.toLowerCase())) return base;
  for (let suffix = 2; ; suffix += 1) {
    const ending = ` ${suffix}`;
    const candidate = `${Array.from(base)
      .slice(0, MAX_LABEL_LENGTH - ending.length)
      .join("")}${ending}`;
    if (!used.has(candidate.toLowerCase())) return candidate;
  }
  return "Workspace";
}

function draftKey(): string {
  return crypto.randomUUID();
}

export function workspaceDraftError(
  workspaces: WorkspaceFolderDraft[],
): string | null {
  const labels = new Set<string>();
  const directories = new Set<string>();
  for (const workspace of workspaces) {
    const label = workspace.label.trim();
    if (!label) return "Every workspace folder needs a name.";
    if (Array.from(label).length > MAX_LABEL_LENGTH) {
      return `A workspace name must be ${MAX_LABEL_LENGTH} characters or fewer.`;
    }
    const labelKey = label.toLowerCase();
    if (labels.has(labelKey)) {
      return "Workspace folder names must be unique for this companion.";
    }
    labels.add(labelKey);

    const directory = workspace.directory.trim();
    if (!directory) return "Every workspace needs a folder.";
    if (directories.has(directory)) {
      return "The same folder cannot be added twice.";
    }
    directories.add(directory);
  }
  return null;
}

export function WorkspaceFolderEditor({
  value,
  disabled = false,
  onChange,
  onError,
}: WorkspaceFolderEditorProps) {
  const validationError = workspaceDraftError(value);

  const pickFolder = async (defaultPath?: string) => {
    const picked = await openFolderDialog({
      directory: true,
      title: "Choose a workspace folder",
      defaultPath,
    });
    return typeof picked === "string" && picked ? picked : null;
  };

  const addFolder = async () => {
    try {
      const directory = await pickFolder(value[value.length - 1]?.directory);
      if (!directory) return;
      onChange([
        ...value,
        {
          key: draftKey(),
          label: availableLabel(directory, value),
          directory,
        },
      ]);
    } catch (error) {
      onError(error);
    }
  };

  const replaceFolder = async (index: number) => {
    try {
      const directory = await pickFolder(value[index]?.directory);
      if (!directory) return;
      onChange(
        value.map((workspace, workspaceIndex) =>
          workspaceIndex === index ? { ...workspace, directory } : workspace,
        ),
      );
    } catch (error) {
      onError(error);
    }
  };

  const moveFolder = (index: number, offset: -1 | 1) => {
    const destination = index + offset;
    if (destination < 0 || destination >= value.length) return;
    const reordered = [...value];
    [reordered[index], reordered[destination]] = [
      reordered[destination],
      reordered[index],
    ];
    onChange(reordered);
  };

  return (
    <div className={styles.editor}>
      <div className={styles.heading}>
        <div>
          <strong>Workspace access</strong>
          <span>Named folders this companion may read and change.</span>
        </div>
        <button
          type="button"
          className={`credential-button credential-button--quiet ${styles.addButton}`}
          disabled={disabled}
          onClick={() => void addFolder()}
        >
          <span aria-hidden="true">+</span>
          Add folder
        </button>
      </div>

      {value.length === 0 ? (
        <div className={styles.empty}>
          <span className={styles.emptyMark} aria-hidden="true">📁</span>
          <div>
            <strong>No workspace access</strong>
            <span>File tools stay hidden until a folder is added.</span>
          </div>
        </div>
      ) : (
        <ol className={styles.list}>
          {value.map((workspace, index) => (
            <li className={styles.item} key={workspace.key}>
              <span className={styles.index} aria-hidden="true">
                {index + 1}
              </span>
              <label className={styles.nameField}>
                <span>Folder name</span>
                <input
                  type="text"
                  required
                  maxLength={MAX_LABEL_LENGTH}
                  disabled={disabled}
                  value={workspace.label}
                  placeholder="e.g. Client website"
                  onChange={(event) =>
                    onChange(
                      value.map((item) =>
                        item.key === workspace.key
                          ? { ...item, label: event.target.value }
                          : item,
                      ),
                    )
                  }
                />
              </label>
              <div className={styles.pathBlock}>
                <span>Folder</span>
                <code title={workspace.directory}>{workspace.directory}</code>
              </div>
              <div className={styles.actions}>
                <button
                  type="button"
                  className={styles.orderButton}
                  disabled={disabled || index === 0}
                  aria-label={`Move ${workspace.label || "workspace"} up`}
                  onClick={() => moveFolder(index, -1)}
                >
                  ↑
                </button>
                <button
                  type="button"
                  className={styles.orderButton}
                  disabled={disabled || index === value.length - 1}
                  aria-label={`Move ${workspace.label || "workspace"} down`}
                  onClick={() => moveFolder(index, 1)}
                >
                  ↓
                </button>
                <button
                  type="button"
                  className={styles.textButton}
                  disabled={disabled}
                  onClick={() => void replaceFolder(index)}
                >
                  Change
                </button>
                <button
                  type="button"
                  className={`${styles.textButton} ${styles.removeButton}`}
                  disabled={disabled}
                  onClick={() =>
                    onChange(value.filter((item) => item.key !== workspace.key))
                  }
                >
                  Remove
                </button>
              </div>
            </li>
          ))}
        </ol>
      )}
      {validationError ? (
        <p className={styles.validation} role="alert">
          {validationError}
        </p>
      ) : null}
    </div>
  );
}
