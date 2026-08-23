import { useShallow } from "zustand/react/shallow";

import { useCompanionStore } from "../workspace/companionStore";
import { ModelPreferenceSelect } from "./ModelPreferenceSelect";

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
      <label className="preference-store__field">
        <span>Default response model</span>
        <ModelPreferenceSelect
          value={userPreferences.defaultModel}
          configuredModels={configuredModels}
          onChange={(preference) => {
            if (preference.mode !== "inherit") void setUserDefaultModel(preference);
          }}
        />
        {preferenceError ? <small>{preferenceError}</small> : null}
      </label>
    </section>
  );
}
