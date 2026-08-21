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

export type UpdateModelCredentialInput =
  | { kind: "saved"; credentialId: string }
  | { kind: "manual"; apiKey: string | null };

export interface CreateConfiguredModelInput {
  providerId: string;
  modelId: string;
  displayName: string;
  credential: ModelCredentialInput;
}

export interface UpdateConfiguredModelInput {
  modelId: string;
  providerId: string;
  providerModelId: string;
  displayName: string;
  credential: UpdateModelCredentialInput;
}

export type ModelChangedEvent =
  | { kind: "created"; model: ConfiguredModel }
  | { kind: "updated"; model: ConfiguredModel }
  | { kind: "deleted"; modelId: string };
