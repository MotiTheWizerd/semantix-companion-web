import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  CreateProviderCredentialInput,
  CredentialChangedEvent,
  ModelProvider,
  ProviderCredential,
} from "./types";

const CREDENTIALS_CHANGED_EVENT = "credentials://changed";

export function listKnownModelProviders(): Promise<ModelProvider[]> {
  return invoke<ModelProvider[]>("list_known_model_providers");
}

export function listProviderCredentials(): Promise<ProviderCredential[]> {
  return invoke<ProviderCredential[]>("list_provider_credentials");
}

export function createProviderCredential(
  input: CreateProviderCredentialInput,
): Promise<ProviderCredential> {
  return invoke<ProviderCredential>("create_provider_credential", { input });
}

export function deleteProviderCredential(credentialId: string): Promise<void> {
  return invoke<void>("delete_provider_credential", { credentialId });
}

export function onCredentialsChanged(
  handler: (event: CredentialChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<CredentialChangedEvent>(CREDENTIALS_CHANGED_EVENT, ({ payload }) => {
    handler(payload);
  });
}
