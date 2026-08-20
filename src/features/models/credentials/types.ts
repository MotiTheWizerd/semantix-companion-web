export interface ModelProvider {
  id: string;
  name: string;
  keyPlaceholder: string;
}

export interface ProviderCredential {
  id: string;
  providerId: string;
  label: string;
  keyHint: string;
  createdAt: number;
  updatedAt: number;
  lastUsedAt: number | null;
}

export interface CreateProviderCredentialInput {
  providerId: string;
  label: string;
  apiKey: string;
}

export type CredentialChangedEvent =
  | { kind: "created"; credential: ProviderCredential }
  | { kind: "deleted"; credentialId: string };

