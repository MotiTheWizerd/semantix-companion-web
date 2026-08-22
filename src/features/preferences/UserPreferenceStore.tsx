import { useShallow } from "zustand/react/shallow";

import { useCompanionStore } from "../workspace/companionStore";
import { modelPreferenceFromValue, modelPreferenceValue } from "./types";

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
          New conversation tabs inherit this model until you choose an explicit override.
        </p>
      </div>
      <label className="preference-store__field">
        <span>Default response model</span>
        <select
          value={modelPreferenceValue(userPreferences.defaultModel)}
          onChange={(event) => {
            const preference = modelPreferenceFromValue(event.target.value);
            if (preference.mode !== "inherit") void setUserDefaultModel(preference);
          }}
        >
          <option value="test">Test stream</option>
          {configuredModels.map((model) => (
            <option key={model.id} value={`configured:${model.id}`}>
              {model.displayName}
            </option>
          ))}
        </select>
        {preferenceError ? <small>{preferenceError}</small> : null}
      </label>
    </section>
  );
}
