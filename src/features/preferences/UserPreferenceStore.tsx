import { useShallow } from "zustand/react/shallow";

import { ModelSelector } from "../models/ModelSelector";
import { useCompanionStore } from "../workspace/companionStore";

export function UserPreferenceStore() {
  const { configuredModels, userPreferences, preferenceError, setUserDefaultModel } =
    useCompanionStore(
      useShallow((state) => ({
        configuredModels: state.configuredModels,
        userPreferences: state.userPreferences,
        preferenceError: state.preferenceError,
        setUserDefaultModel: state.setUserDefaultModel,
      })),
    );

  return (
    <section className="preference-store" aria-labelledby="default-model-heading">
      <div>
        <p className="credential-store__eyebrow">User preference</p>
        <h2 id="default-model-heading">Default model</h2>
        <p className="credential-store__description">
          Companions inherit this model until one is given a model of its own.
        </p>
      </div>
      <div className="preference-store__field">
        <span>Default response model</span>
        <ModelSelector
          value={userPreferences.defaultModel}
          configuredModels={configuredModels}
          userPreferences={userPreferences}
          ariaLabel="Default response model"
          onChange={(preference) => {
            if (preference.mode !== "inherit") void setUserDefaultModel(preference);
          }}
        />
        {preferenceError ? <small>{preferenceError}</small> : null}
      </div>
    </section>
  );
}
