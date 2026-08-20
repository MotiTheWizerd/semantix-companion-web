export type ModelCredentialKind = "saved" | "manual";

export interface ConfiguredModel {
  id: string;
  providerId: string;
  modelId: string;
  displayName: string;
  credentialId: string | null;
  credentialKind: ModelCredentialKind;
  credentialLabel: string;
  keyHint: string;
  createdAt: number;
  updatedAt: number;
}

export type ModelCredentialInput =
  | { kind: "saved"; credentialId: string }
  | { kind: "manual"; apiKey: string };

export interface CreateConfiguredModelInput {
  providerId: string;
  modelId: string;
  displayName: string;
  credential: ModelCredentialInput;
}

export type ModelChangedEvent =
  | { kind: "created"; model: ConfiguredModel }
  | { kind: "deleted"; modelId: string };

