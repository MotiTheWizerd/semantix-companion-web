// The style harvest — from "I miss how it used to talk" to thirty real
// exchanges in that voice, pruned by hand.
//
// Three moments, mirroring the history import the user already knows:
//
//   1. DROP the export — same zips, same folders, same sniffing. A user who
//      already imported history drops the same file again.
//   2. CHOOSE the voice — a ChatGPT export names who really answered every
//      message, so "GPT-4o spoke in 1,823 chats" is a real menu. A Claude
//      export never says which model spoke, so there the menu is TIME:
//      "Claude, during 2024" is how that format spells a model era.
//   3. PRUNE the preview — every exchange offered is shown whole and can be
//      struck out. These are the user's own private words about to ride a
//      system prompt; nothing is kept that they did not look at.

import { useMemo, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import { harvestStyleExemplars, inspectStyleSource } from "./styleService";
import type { HarvestedPair, StyleSourceInspection } from "./types";

interface StyleHarvestWizardProps {
  onHarvested: (pairs: HarvestedPair[], suggestedName: string) => void;
  onClose: () => void;
}

const TARGET_CHOICES = [30, 60, 100] as const;

/** Model slugs worth offering — a voice needs a real corpus behind it. */
const MIN_CHATS_FOR_A_VOICE = 5;

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "The harvest ran into a problem.";
}

function monthYear(ms: number): string {
  if (ms <= 0) return "";
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    year: "numeric",
  });
}

/** The calendar years an export spans — the Claude era menu. */
function yearsOf(inspection: StyleSourceInspection): number[] {
  if (inspection.earliestMs <= 0 || inspection.latestMs <= 0) return [];
  const first = new Date(inspection.earliestMs).getFullYear();
  const last = new Date(inspection.latestMs).getFullYear();
  const years: number[] = [];
  for (let year = first; year <= last; year += 1) years.push(year);
  return years;
}

export function StyleHarvestWizard({ onHarvested, onClose }: StyleHarvestWizardProps) {
  const [path, setPath] = useState<string | null>(null);
  const [inspection, setInspection] = useState<StyleSourceInspection | null>(null);
  const [modelSlug, setModelSlug] = useState<string | null>(null);
  const [year, setYear] = useState<number | null>(null);
  const [target, setTarget] = useState<number>(TARGET_CHOICES[0]);
  const [pairs, setPairs] = useState<HarvestedPair[] | null>(null);
  const [matchedPairs, setMatchedPairs] = useState(0);
  const [struck, setStruck] = useState<Set<number>>(new Set());
  const [isBusy, setIsBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const offeredModels = useMemo(
    () =>
      (inspection?.models ?? []).filter(
        (model) => model.chatCount >= MIN_CHATS_FOR_A_VOICE,
      ),
    [inspection],
  );

  const keptCount = (pairs?.length ?? 0) - struck.size;

  const inspect = async (picked: string) => {
    setError(null);
    setIsBusy(true);
    try {
      const result = await inspectStyleSource(picked);
      setPath(picked);
      setInspection(result);
      setModelSlug(result.models[0]?.slug ?? null);
      setYear(null);
      setPairs(null);
      setStruck(new Set());
    } catch (inspectError) {
      setError(errorMessage(inspectError));
    } finally {
      setIsBusy(false);
    }
  };

  const pickZip = async () => {
    const picked = await openFileDialog({
      title: "Choose your export (.zip or conversations.json)",
      filters: [{ name: "Chat export", extensions: ["zip", "json"] }],
    });
    if (typeof picked === "string" && picked) await inspect(picked);
  };

  const pickFolder = async () => {
    const picked = await openFileDialog({
      directory: true,
      title: "Choose the folder your export unzipped into",
    });
    if (typeof picked === "string" && picked) await inspect(picked);
  };

  const harvest = async () => {
    if (!path || !inspection) return;
    setError(null);
    setIsBusy(true);
    try {
      const window =
        inspection.source === "claude" && year !== null
          ? {
              fromMs: Date.UTC(year, 0, 1),
              toMs: Date.UTC(year + 1, 0, 1) - 1,
            }
          : {};
      const result = await harvestStyleExemplars({
        path,
        modelSlug: inspection.source === "chatgpt" ? modelSlug : null,
        target,
        ...window,
      });
      setPairs(result.pairs);
      setMatchedPairs(result.matchedPairs);
      setStruck(new Set());
      if (result.pairs.length === 0) {
        setError(
          "No usable exchanges matched — try another model, a wider time range, or a different export.",
        );
      }
    } catch (harvestError) {
      setError(errorMessage(harvestError));
    } finally {
      setIsBusy(false);
    }
  };

  const toggleStruck = (index: number) => {
    setStruck((current) => {
      const next = new Set(current);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const finish = () => {
    if (!pairs || !inspection) return;
    const kept = pairs.filter((_, index) => !struck.has(index));
    const suggestedName =
      inspection.source === "chatgpt"
        ? (modelSlug ?? "ChatGPT")
        : year !== null
          ? `Claude, ${year}`
          : "Claude";
    onHarvested(kept, suggestedName);
  };

  return (
    <div className="style-harvest">
      <div className="credential-form__heading">
        <div>
          <h4>Harvest a voice from your export</h4>
          <p>
            Drop the same Claude or ChatGPT export you use for history import.
            Everything is read on this computer — nothing is uploaded.
          </p>
        </div>
        <button
          className="credential-form__close"
          type="button"
          aria-label="Close the harvest"
          onClick={onClose}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m7 7 10 10M17 7 7 17" />
          </svg>
        </button>
      </div>

      {inspection === null ? (
        <div className="style-harvest__pickers">
          <button
            className="credential-button credential-button--primary"
            type="button"
            disabled={isBusy}
            onClick={() => void pickZip()}
          >
            {isBusy ? "Reading…" : "Choose the downloaded file"}
          </button>
          <button
            className="credential-button credential-button--quiet"
            type="button"
            disabled={isBusy}
            onClick={() => void pickFolder()}
          >
            Choose an unzipped folder
          </button>
        </div>
      ) : pairs === null ? (
        <div className="style-harvest__choose">
          <p>
            {inspection.source === "chatgpt" ? "ChatGPT" : "Claude"} export —{" "}
            {inspection.conversationCount.toLocaleString()} conversations
            {monthYear(inspection.earliestMs)
              ? `, ${monthYear(inspection.earliestMs)} → ${monthYear(inspection.latestMs)}`
              : ""}
          </p>

          {inspection.source === "chatgpt" ? (
            offeredModels.length > 0 ? (
              <div role="radiogroup" aria-label="Which voice" className="style-harvest__models">
                {offeredModels.map((model) => (
                  <label key={model.slug}>
                    <input
                      type="radio"
                      name="style-harvest-model"
                      checked={modelSlug === model.slug}
                      onChange={() => setModelSlug(model.slug)}
                    />
                    <span>
                      <strong>{model.slug}</strong> — spoke in{" "}
                      {model.chatCount.toLocaleString()} chats
                    </span>
                  </label>
                ))}
              </div>
            ) : (
              <p>This export doesn't say which model spoke — the whole archive will be used.</p>
            )
          ) : (
            <div role="radiogroup" aria-label="Which era" className="style-harvest__models">
              <label>
                <input
                  type="radio"
                  name="style-harvest-era"
                  checked={year === null}
                  onChange={() => setYear(null)}
                />
                <span>
                  <strong>All years</strong> — Claude exports don't name models,
                  so the era is how you choose a voice
                </span>
              </label>
              {yearsOf(inspection).map((candidate) => (
                <label key={candidate}>
                  <input
                    type="radio"
                    name="style-harvest-era"
                    checked={year === candidate}
                    onChange={() => setYear(candidate)}
                  />
                  <span>
                    <strong>{candidate}</strong> only
                  </span>
                </label>
              ))}
            </div>
          )}

          <label className="style-harvest__target">
            <span>Exchanges to gather</span>
            <select
              value={target}
              onChange={(event) => setTarget(Number(event.target.value))}
            >
              {TARGET_CHOICES.map((choice) => (
                <option key={choice} value={choice}>
                  {choice}
                  {choice === 30 ? " — recommended" : ""}
                </option>
              ))}
            </select>
          </label>

          <div className="credential-form__actions">
            <button
              className="credential-button credential-button--quiet"
              type="button"
              disabled={isBusy}
              onClick={() => {
                setPath(null);
                setInspection(null);
                setError(null);
              }}
            >
              Back
            </button>
            <button
              className="credential-button credential-button--primary"
              type="button"
              disabled={isBusy || (inspection.source === "chatgpt" && offeredModels.length > 0 && !modelSlug)}
              onClick={() => void harvest()}
            >
              {isBusy ? "Gathering…" : "Gather exchanges"}
            </button>
          </div>
        </div>
      ) : (
        <div className="style-harvest__preview">
          <p>
            {matchedPairs.toLocaleString()} exchanges matched; here are the{" "}
            {pairs.length} most useful. <strong>Read them before keeping them</strong> —
            these are your own conversations, and every kept exchange becomes part
            of how this style speaks. Strike out anything private or off-voice.
          </p>
          <ul className="style-harvest__pairs">
            {pairs.map((pair, index) => {
              const isStruck = struck.has(index);
              return (
                <li key={index} className={isStruck ? "is-struck" : undefined}>
                  <div className="style-harvest__pair-meta">
                    <span>
                      {pair.era ?? "undated"} · {pair.chatTitle || "untitled chat"}
                    </span>
                    <button
                      type="button"
                      className="credential-button credential-button--quiet"
                      onClick={() => toggleStruck(index)}
                    >
                      {isStruck ? "Keep it" : "Strike out"}
                    </button>
                  </div>
                  <details>
                    <summary>{pair.userText.slice(0, 140)}</summary>
                    <p className="style-harvest__pair-user">{pair.userText}</p>
                    <p className="style-harvest__pair-voice">{pair.companionText}</p>
                  </details>
                </li>
              );
            })}
          </ul>
          <div className="credential-form__actions">
            <button
              className="credential-button credential-button--quiet"
              type="button"
              onClick={() => {
                setPairs(null);
                setError(null);
              }}
            >
              Back
            </button>
            <button
              className="credential-button credential-button--primary"
              type="button"
              disabled={keptCount === 0}
              onClick={finish}
            >
              Use {keptCount} exchange{keptCount === 1 ? "" : "s"}
            </button>
          </div>
        </div>
      )}

      {error ? (
        <p className="credential-store__error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
