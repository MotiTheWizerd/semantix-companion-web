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

export interface UpdateProviderCredentialInput {
  credentialId: string;
  providerId: string;
  label: string;
  apiKey: string | null;
}

export type CredentialChangedEvent =
  | { kind: "created"; credential: ProviderCredential }
  | { kind: "updated"; credential: ProviderCredential }
  | { kind: "deleted"; credentialId: string };
