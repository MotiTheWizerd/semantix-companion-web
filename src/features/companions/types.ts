// The companion contract, spoken by both halves of the app.
//
// A companion is a name (optional) plus ONE private memory. `memoryAgentName`
// is that memory's identity on the organ roster — assigned by Rust when the
// record is made and never reassigned, so a rename cannot orphan what a
// companion remembers.

export interface Companion {
  id: string;
  /** `null` = unnamed. A blank name is a real, supported state, not an error. */
  name: string | null;
  /** This companion's private memory on the organ roster. Read-only here. */
  memoryAgentName: string;
  /** The seeded companion: owns the memory carved before companions existed,
   *  and cannot be deleted. */
  isBuiltIn: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface CreateCompanionInput {
  name: string | null;
}

export interface UpdateCompanionInput {
  companionId: string;
  name: string | null;
}

export type CompanionChangedEvent =
  | { kind: "created"; companion: Companion }
  | { kind: "updated"; companion: Companion }
  | { kind: "deleted"; companionId: string };

export const UNNAMED_COMPANION_LABEL = "Unnamed companion";

/** The one place that decides how an unnamed companion reads on screen. */
export function companionLabel(companion: Companion): string {
  return companion.name ?? UNNAMED_COMPANION_LABEL;
}
